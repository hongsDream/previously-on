# Compatibility

The last public tag is `0.1.0-alpha.1`; the alpha.2 code line was merged to `main` without a public
version tag. The `0.1.0-alpha.3` source preview adds task grouping, an evidence-backed relationship graph,
beta AI candidate refresh, and same-device local agent lineage. It is not a new public release
claim: this work does not create a tag, GitHub Release, or crates.io publication. Historical
evidence cannot be relabelled for `alpha.3`, and the existing 60 authenticated compatibility
workflows are not rerun by this product implementation task.

## Alpha.3 App Server and Unreleased desktop boundary

The release regression suite uses fake App Servers to verify the documented experimental request
shapes without starting a real model turn. `permissionProfile/list` must report the managed
`previously-input-only` profile as allowed before Refresh is enabled. `thread/start` then receives
named `permissions` and approval `never`, never both `permissions` and legacy `sandbox`, while
`turn/start` receives the strict candidate `outputSchema`. The configured default model is
inherited rather than hard-coded. A single initialized client performs profile verification and
execution from a minimal allowlisted environment; concurrent operation/candidate claims are
transactional and cannot fan out duplicate calls.

App Server readiness is capability-based rather than version-gated. The detected CLI version and
the `0.144.3`/`0.144.2` test provenance are reported as warnings only. Core import, continuation,
and experimental refresh are evaluated separately from the installed binary's official
`generate-json-schema` output; a bounded read-only `thread/list` also verifies the live result
shape. `supportedMethods` contains only schema-confirmed methods plus methods observed through that
safe runtime probe. `previously doctor` performs these checks without creating a thread, starting a
turn, or invoking a model.

Agent import requests the documented interactive and sub-agent `sourceKinds` with pagination and
uses experimental `parentThreadId` only when it is present. Unsupported methods, a blocked
profile, malformed pagination, cross-repository threads, or missing parents degrade or fail closed;
`thread/read` must also return the requested ID and registered concrete worktree, and unsafe file paths
are dropped. These cases are never converted into inferred compatibility. The Unreleased
continuation path uses the documented `codex://threads/<thread-id>` link only after `turn/start`
succeeds; automatic opening is best-effort and the review UI retains the exact link plus Copy ID
as recovery actions. Recovery resumes the recorded thread, reads it back, and validates its ID and
concrete worktree before starting another turn.

Immediately before a fresh or recovered continuation starts a turn, PreviouslyOn reevaluates the
exact source worktree, task changes, active Contract relevance, content fingerprint, and retained
test evidence. The internal handoff wraps the unchanged public Context Pack together with that
Contract evaluation. It does not execute required tests. A blocked Contract is handed off for the
new task to resolve, while invalid Contract JSON, repository mismatch, fingerprint failure, or an
oversized handoff records a durable failure before any new turn is started.

An actual AI calibration/model call remains unavailable in this non-interactive release task. It
requires a compatible App Server, a verified permission profile, and fresh user approval at the
time of execution. No token, model, or latency metric is fabricated when the App Server does not
expose it.

The contract references are the official [App Server API overview](https://learn.chatgpt.com/docs/app-server#api-overview),
[thread start/resume](https://learn.chatgpt.com/docs/app-server#start-or-resume-a-thread),
[turn start](https://learn.chatgpt.com/docs/app-server#start-a-turn), and
[permissions](https://learn.chatgpt.com/docs/permissions) documentation, plus Codex's documented
[supported links](https://learn.chatgpt.com/docs/reference/commands#supported-links). No private
API or undocumented desktop deep link is used.

The repository contains a local mapped-regression matrix with five categories and six entries each:

- lifecycle
- event reconstruction
- concurrency and Git
- App Server
- setup, privacy, and recovery

The canonical mapping is `fixtures/compatibility/scenarios.json`. Every entry points to one exact
Rust test target and test name. The driver requires the named filter to resolve to exactly one
test, executes that test separately for each Codex version, and records the actual per-row exit
outcome. A missing or misspelled filter fails instead of being counted as a pass. This is
regression traceability, not evidence that 30 real Codex workflows ran.

## Run the release gate

```sh
scripts/run-compatibility.sh --codex-app-current <build> --codex-app-previous <build>
```

By default the driver queries npm, selects the two stable CLI versions, installs them in a temporary
directory, validates their official experimental JSON schemas, then probes documented `initialize`
and read-only `thread/list` App Server shapes. It requires core import, continuation, and
experimental refresh schema contracts to be complete before running all 30 mapped local
regressions for both version slots. It writes exact versions and results to
`outputs/mapped-compatibility-results.json`.

The result deliberately separates:

- `liveAppServerSchemaProbes`: real initialization/list schema probes against both installed binaries;
- `localMappedRegression`: repository regression suites associated with the 30 fixture rows;
- `liveCodexWorkflowMatrix`: actual multi-turn Codex workflows, currently `not_run`;
- `transparentCaptureReleaseGate`: always ineligible from this driver alone.

Passing the current script cannot be cited as transparent-capture compatibility. It validates the explicit wrapper/import fallback and local regression surface only.

## Separately supplied release evidence

The tag workflow does not consume the mapped-regression output as release eligibility. Before a
tag can build even a draft release, the protected `release-compatibility` environment must supply:

- `LIVE_COMPATIBILITY_BUNDLE_URL`: an HTTPS URL for the independently produced live JSON plus
  retained-evidence tarball;
- `LIVE_COMPATIBILITY_BUNDLE_SHA256`: its reviewed 64-character SHA-256 digest.

`scripts/validate-live-compatibility.mjs` requires the artifact to match the tagged commit and
product version, the npm registry's current and previous stable Codex versions, and all 30
scenario IDs for each CLI version. Every scenario must be a real passing workflow with evidence
hashes and reconstructed prompt, assistant final, file-change tool, test command, and stable
source IDs. The artifact must also record zero data-loss events. Serious stale applications are
never a hardcoded counter: release eligibility requires a separately retained evaluator JSON file
whose hash, commit, version, evaluated count, and zero-incident result are verified. Omitting that
file records `status: "unmeasured"` and keeps automatic release eligibility false. The mapped
driver emits a different `evidenceClass`, has an ineligible release gate, and cannot pass this
validator.

See [Live compatibility evidence contract](live-compatibility-evidence.md) for the accepted JSON
shape and privacy boundary.

The tag workflow downloads the pinned artifact only after environment approval, validates it,
uploads it into the tag workflow under an `eligible-live-compatibility-*` name, and revalidates it
before building the draft. The compatibility evidence is not silently added to public release
assets; its public-disclosure review is separate from release eligibility.

For an offline rehearsal with two already-installed binaries:

```sh
scripts/run-compatibility.sh \
  --latest-bin /path/to/latest/codex \
  --previous-bin /path/to/previous/codex \
  --output /tmp/previously-mapped-compatibility.json
```

The two binaries must report different versions. An offline rehearsal is useful for repeatability, but the public-release result must use the automatically resolved npm versions.

## Coverage rules

- A deleted App Server thread is `skipped`; its JSON-RPC `code` and `data` remain in the import notice.
- A compacted, incomplete, unknown-item, or version-mismatched thread is imported as `degraded` and remains untrusted data.
- Repeated cursors and malformed pages stop pagination safely. Previously validated pages remain available and coverage becomes `degraded`.
- When App Server omits a stable session, turn, or item ID, PreviouslyOn assigns a UUID and marks coverage `degraded`. Payload hashes are never used as substitute source IDs.
- The supported path requires stable IDs for prompts, assistant finals, file-change tools, and test commands. If the live release matrix cannot reconstruct all four, transparent capture must not be advertised as supported.

Transparent capture may only become supported after separate evidence runs 30 real workflows on each required CLI version and reconstructs the prompt, assistant final, file-change tool, and test command with stable linked source IDs. Until then the release must document `explicit_run_and_import` as its support mode.

## Authenticated live workflow harness

The producer for that separate evidence is `scripts/compatibility/live-harness.mjs`. It performs
30 real two-turn workflows with a supplied latest Codex binary and the same 30 workflows with a
supplied previous binary. These 60 authenticated workflows may incur model charges and are never
run by normal CI.

Validate the full command and fixture schema without authentication, network calls, or model use:

```sh
node scripts/compatibility/live-harness.mjs \
  --dry-run \
  --output /tmp/previously-live-plan.json
node --test scripts/compatibility/live-harness.test.mjs
```

The release owner runs the live gate manually from a clean Apple Silicon checkout. All paths are
absolute. Current Codex App evidence and either prior-build evidence or a documented unavailable
result are supplied as sanitized JSON files. Each digest must match a retained file copied into
the evidence bundle, and resume revalidates those bytes. A current App result marked `degraded`
is preserved but cannot make the release eligible:

```sh
node scripts/compatibility/live-harness.mjs \
  --latest-bin /absolute/path/to/latest/codex \
  --previous-bin /absolute/path/to/previous/codex \
  --previously-bin /absolute/path/to/previously \
  --mapped-artifact /absolute/path/to/mapped-compatibility-results.json \
  --codex-home /absolute/path/to/authenticated/CODEX_HOME \
  --model gpt-5.6-sol \
  --reasoning-effort medium \
  --codex-app-current-build <build> \
  --codex-app-current-evidence /absolute/path/to/current-app-evidence.json \
  --codex-app-current-evidence-sha256 <sha256> \
  --codex-app-previous-build <build> \
  --codex-app-previous-evidence /absolute/path/to/previous-app-evidence.json \
  --codex-app-previous-evidence-sha256 <sha256> \
  --output /absolute/path/to/live-compatibility.json \
  --confirm RUN_60_AUTHENTICATED_CODEX_WORKFLOWS
```

If an authenticated matrix is interrupted, rerun the same command with `--resume`. The harness
opens the existing output and evidence directory, verifies each completed scenario's retained
evidence, and skips only checkpoints that are still valid. Resume fails closed if the Git commit,
PreviouslyOn or Codex binary digest, resolved Codex version, fixture contract, scenario matrix,
mapped artifact, or App evidence binding differs. Normal live and `--resume` modes reject
`--stale-evaluation-*` arguments and remain `unmeasured`; only finalize may attach the evaluator.
The fixture contract also binds `gpt-5.6-sol`, medium reasoning, `workspace-write`, strict config,
and a 600-second per-turn timeout. Both the initial and resumed turns receive the same explicit
settings, and resume rejects checkpoints produced under any different execution policy.
In case any other immutable binding differs, choose a new output path and
produce a new artifact; do not copy completed rows into a differently bound matrix.

When all 60 workflows are complete, produce the serious-stale evaluator from those retained
results and attach it with the one-way finalize mode. Repeat the same binary, mapped-artifact,
and Codex App arguments from the original run; finalize reopens all 60 scenario files and hashes,
revalidates every immutable binding, and never starts an authenticated Codex workflow. It only
uses the supplied binaries for version and command-shape identity checks:

```sh
node scripts/compatibility/live-harness.mjs \
  --finalize-stale-evaluation \
  --latest-bin /absolute/path/to/latest/codex \
  --previous-bin /absolute/path/to/previous/codex \
  --previously-bin /absolute/path/to/previously \
  --mapped-artifact /absolute/path/to/mapped-compatibility-results.json \
  --model gpt-5.6-sol \
  --reasoning-effort medium \
  --codex-app-current-build <build> \
  --codex-app-current-evidence /absolute/path/to/current-app-evidence.json \
  --codex-app-current-evidence-sha256 <sha256> \
  --codex-app-previous-build <build> \
  --codex-app-previous-evidence /absolute/path/to/previous-app-evidence.json \
  --codex-app-previous-evidence-sha256 <sha256> \
  --stale-evaluation-artifact /absolute/path/to/stale-evaluation.json \
  --stale-evaluation-sha256 <sha256> \
  --output /absolute/path/to/live-compatibility.json \
  --bundle /absolute/path/to/live-compatibility-finalized.tar.gz
```

Finalize is allowed only from a complete, lossless, release-ineligible artifact whose stale
status is still `unmeasured`. The evaluator must bind the same product version and Git commit and
cover all 60 workflows. A measured artifact cannot be replaced, even by a different evaluator;
an incomplete matrix, modified scenario/mapped/App evidence, changed binary/version/runner, or
evaluator mismatch fails closed. If finalize is interrupted after retaining the evaluator but
before updating the artifact, repeating the exact command accepts only the identical evaluator
bytes. The default finalized bundle name is `live-compatibility.json.finalized.tar.gz`, so it does
not overwrite the original unmeasured bundle.

The initial 60-run command intentionally exits non-zero after retaining a complete matrix when
stale evaluation is still `unmeasured`; this is a release-gate result, not a request to rerun the
workflows. Generate the evaluator from those retained results and use finalize as shown above.

The source `CODEX_HOME` is never modified. Only its `auth.json` is copied into a permission-limited
temporary home; existing config, Hooks, rules, plugins, memories, and sessions are excluded. Each
workflow uses a unique temporary Git repository and invokes `codex exec`, `codex exec resume`,
PreviouslyOn setup/run/import/export, and uninstall. Synthetic prompts edit only `state.txt` and
run the local `verify.sh` fixture. The separately generated mapped artifact binds every scenario
ID to its reviewed Rust target, filter, and expectation at the same commit and CLI versions; the
live edit itself is not mislabeled as a category-specific fault injection.

The harness records ground-truth filesystem and Git hashes, Codex JSONL observation hashes, final
message hashes, and hashes of App Server import coverage and the PreviouslyOn export. It does not
write the raw Codex JSONL stream to the retained evidence directory. Eligibility remains false
unless all 60 workflows reconstruct both user prompts, both assistant finals, paired file-change
tools, both test commands, the observed model identity, and unique stable source IDs linked to the
resumed session. Missing auth,
dirty source, incomplete evidence, or any failed workflow exits non-zero. The long confirmation
phrase prevents accidental paid execution.
