import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  buildDryRunPlan,
  buildWorkflowFixture,
  computeEligibility,
  packageEvidenceBundle,
  validateMappedArtifact,
  validateFixtureContract,
  verifyScenarioEvidence,
} from "./live-harness.mjs";

const matrix = JSON.parse(readFileSync(new URL("../../fixtures/compatibility/scenarios.json", import.meta.url), "utf8"));
const contract = JSON.parse(readFileSync(new URL("../../fixtures/compatibility/live-workflow-contract.json", import.meta.url), "utf8"));

test("live fixture contract expands to 60 two-turn workflow slots without executing sessions", () => {
  const validated = validateFixtureContract(matrix, contract);
  assert.equal(validated.scenarios.length, 30);
  assert.deepEqual(Object.values(validated.categories).sort(), [6, 6, 6, 6, 6]);
  const plan = buildDryRunPlan(matrix, contract);
  assert.equal(plan.requiredRuns, 60);
  assert.equal(plan.paidSessionsExecuted, 0);
  assert.equal(plan.transparentCaptureReleaseGate.eligible, false);
  assert.equal(plan.workflows.length, 30);
  assert.match(plan.workflows[0].initialPrompt, /apply_patch/);
  assert.match(plan.workflows[0].resumePrompt, /verify\.sh/);
});

test("live workflow markers never collide with the secret redaction corpus", () => {
  const scenario = matrix.scenarios.find(({ id }) => id === "privacy-secret-corpus");
  const fixture = buildWorkflowFixture(scenario, contract);
  assert.equal(fixture.id, "privacy-secret-corpus");
  assert.doesNotMatch(fixture.initialPrompt, /secret/i);
  assert.match(fixture.initialPrompt, /privacy-redacted-corpus/);
});

test("evidence verifier requires prompt final paired file tools tests and stable linked IDs", () => {
  const fixture = buildWorkflowFixture(matrix.scenarios[0], contract);
  const sessionId = "019f0000-0000-7000-8000-000000000001";
  let source = 0;
  const event = (kind, payload, sourceId = null) => ({
    kind,
    payload,
    session_id: sessionId,
    source_id: sourceId ?? `src-${(++source).toString(16).padStart(64, "0")}`,
    coverage: { status: "complete", missing: [] },
  });
  const filePair = (value, toolUseId) => [
    event("tool_started", { tool_use_id: toolUseId, tool_name: "apply_patch", tool_input: { command: `*** Update File: state.txt\n+${value}` } }),
    event("tool_finished", { tool_use_id: toolUseId, tool_name: "apply_patch", tool_input: { command: `*** Update File: state.txt\n+${value}` }, tool_response: { content: "Done" } }),
  ];
  const events = [
    event("user_prompt", { prompt: fixture.initialPrompt }),
    ...filePair(fixture.INITIAL_VALUE, "tool-initial"),
    event("tool_finished", { tool_use_id: "test-initial", tool_input: { command: fixture.initialTestCommand }, tool_response: { exit_code: 0 } }),
    event(
      "assistant_final",
      { last_assistant_message: fixture.INITIAL_FINAL },
      `codex-app-server:thread:${sessionId}:item:final-initial:assistant-final`,
    ),
    event("session_stopped", { last_assistant_message: fixture.INITIAL_FINAL }),
    event("user_prompt", { prompt: fixture.resumePrompt }),
    ...filePair(fixture.RESUME_VALUE, "tool-resume"),
    event("tool_finished", { tool_use_id: "test-resume", tool_input: { command: fixture.resumeTestCommand }, tool_response: { exit_code: 0 } }),
    event(
      "assistant_final",
      { last_assistant_message: fixture.RESUME_FINAL },
      `codex-app-server:thread:${sessionId}:item:final-resume:assistant-final`,
    ),
    event("session_stopped", { last_assistant_message: fixture.RESUME_FINAL }),
  ];
  events.splice(
    1,
    0,
    event("tool_started", {
      tool_use_id: "failed-tool-initial",
      tool_name: "apply_patch",
      tool_input: { command: `*** Update File: state.txt\n+${fixture.INITIAL_VALUE}` },
    }),
  );
  const verified = verifyScenarioEvidence({
    fixture,
    exportData: { canonical_events: events },
    sessionId,
    initialFinalText: fixture.INITIAL_FINAL,
    resumeFinalText: fixture.RESUME_FINAL,
    initialContent: `${fixture.INITIAL_VALUE}\n`,
    resumeContent: `${fixture.RESUME_VALUE}\n`,
    gitStatus: " M state.txt\n",
  });
  assert.equal(verified.passed, true);
  assert.deepEqual(verified.reconstruction, {
    userPrompt: true,
    assistantFinal: true,
    fileChangeTool: true,
    testCommand: true,
    stableSourceIds: true,
  });
  events[0].coverage.missing.push("stable_source_id");
  assert.equal(
    verifyScenarioEvidence({
      fixture,
      exportData: { canonical_events: events },
      sessionId,
      initialFinalText: fixture.INITIAL_FINAL,
      resumeFinalText: fixture.RESUME_FINAL,
      initialContent: `${fixture.INITIAL_VALUE}\n`,
      resumeContent: `${fixture.RESUME_VALUE}\n`,
      gitStatus: " M state.txt\n",
    }).reconstruction.stableSourceIds,
    false,
  );
});

test("release eligibility is true only for two complete distinct 30-scenario runs", () => {
  const ids = matrix.scenarios.map((scenario) => scenario.id);
  const scenarioResults = matrix.scenarios.map((scenario) => ({
    id: scenario.id,
    status: "passed",
    reconstruction: Object.fromEntries(contract.requiredReconstruction.map((field) => [field, true])),
    scenarioAssertion: {
      status: "passed",
      testTarget: scenario.testTarget,
      testFilter: scenario.testFilter,
      expectation: scenario.expectation,
      mappedArtifactSha256: "c".repeat(64),
    },
    evidenceSha256: "a".repeat(64),
  }));
  const artifact = {
    codexCli: {
      runs: [
        { role: "latest", version: "2.0.0", scenarios: structuredClone(scenarioResults) },
        { role: "previous", version: "1.9.0", scenarios: structuredClone(scenarioResults) },
      ],
    },
    releaseEligibility: { dataLossEvents: 0 },
    codexApp: {
      current: { status: "passed", build: "current-build", evidenceSha256: "b".repeat(64) },
      previous: { status: "unavailable", reason: "vendor build unavailable", checkedAt: "2026-07-13T00:00:00Z" },
    },
  };
  assert.equal(computeEligibility(artifact, ids), true);
  artifact.codexCli.runs[1].scenarios[12].reconstruction.testCommand = false;
  assert.equal(computeEligibility(artifact, ids), false);
});

test("mapped artifact validation binds all scenario assertions to the same commit and CLI versions", () => {
  const directory = mkdtempSync(join(tmpdir(), "previously-mapped-artifact-test-"));
  try {
    const matrixBytes = readFileSync(new URL("../../fixtures/compatibility/scenarios.json", import.meta.url));
    const ids = matrix.scenarios.map((scenario) => scenario.id);
    const artifact = {
      schemaVersion: 1,
      product: "PreviouslyOn",
      evidenceClass: "local_mapped_regression_plus_live_app_server_schema_probe",
      productVersion: "0.1.0-alpha.1",
      supportMode: "explicit_run_and_import",
      gitCommit: "d".repeat(40),
      gitTreeState: "clean",
      scenarioMatrixSha256: createHash("sha256").update(matrixBytes).digest("hex"),
      localMappedRegression: {
        scenarioCount: 30,
        runs: [
          { codexVersionSlot: "2.0.0", passed: ids, failed: [] },
          { codexVersionSlot: "1.9.0", passed: ids, failed: [] },
        ],
      },
      liveCodexWorkflowMatrix: { status: "not_run" },
      transparentCaptureReleaseGate: { eligible: false },
    };
    const path = join(directory, "mapped.json");
    writeFileSync(path, `${JSON.stringify(artifact)}\n`);
    const validated = validateMappedArtifact(
      path,
      matrix.scenarios,
      "d".repeat(40),
      "0.1.0-alpha.1",
      "2.0.0",
      "1.9.0",
    );
    assert.equal(validated.status, "separate_artifact_passed");
    assert.match(validated.artifactSha256, /^[0-9a-f]{64}$/);
    artifact.localMappedRegression.runs[1].failed.push(ids[0]);
    writeFileSync(path, `${JSON.stringify(artifact)}\n`);
    assert.throws(
      () => validateMappedArtifact(path, matrix.scenarios, "d".repeat(40), "0.1.0-alpha.1", "2.0.0", "1.9.0"),
      /did not pass/,
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("release evidence bundling carries the top artifact and retained verdict directory", () => {
  const directory = mkdtempSync(join(tmpdir(), "previously-live-bundle-test-"));
  try {
    const artifact = join(directory, "live-compatibility.json");
    const evidence = join(directory, "live-compatibility.json.evidence");
    const bundle = join(directory, "live-compatibility.tar.gz");
    mkdirSync(join(evidence, "latest", "scenario"), { recursive: true });
    writeFileSync(artifact, "{}\n");
    writeFileSync(join(evidence, "latest", "scenario", "evidence.json"), "{}\n");
    assert.equal(packageEvidenceBundle(artifact, evidence, bundle), bundle);
    const listed = spawnSync("tar", ["-tzf", bundle], { encoding: "utf8" });
    assert.equal(listed.status, 0, listed.stderr);
    assert.match(listed.stdout, /live-compatibility\.json\n/);
    assert.match(listed.stdout, /live-compatibility\.json\.evidence\/latest\/scenario\/evidence\.json/);
    assert.throws(() => packageEvidenceBundle(artifact, evidence, bundle), /refusing to overwrite/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("release validator accepts only a complete bound 60-run artifact", () => {
  const directory = mkdtempSync(join(tmpdir(), "previously-live-validator-test-"));
  try {
    const commit = "e".repeat(40);
    const evidenceRoot = join(directory, "live-compatibility.json.evidence");
    mkdirSync(evidenceRoot, { recursive: true });
    const mappedEvidence = {
      schemaVersion: 1,
      gitCommit: commit,
      localMappedRegression: { scenarioCount: 30 },
    };
    const mappedBytes = `${JSON.stringify(mappedEvidence)}\n`;
    const mappedPath = join(evidenceRoot, "mapped-compatibility-results.json");
    writeFileSync(mappedPath, mappedBytes);
    const mappedSha = createHash("sha256").update(mappedBytes).digest("hex");
    const scenarios = matrix.scenarios.map((scenario) => ({
      id: scenario.id,
      category: scenario.category,
      status: "passed",
      reconstruction: Object.fromEntries(contract.requiredReconstruction.map((field) => [field, true])),
      scenarioAssertion: {
        status: "passed",
        testTarget: scenario.testTarget,
        testFilter: scenario.testFilter,
        expectation: scenario.expectation,
        mappedArtifactSha256: mappedSha,
      },
      evidenceSha256: "a".repeat(64),
    }));
    const artifact = {
      schemaVersion: 1,
      evidenceClass: "live_codex_workflow_matrix",
      product: "PreviouslyOn",
      productVersion: "0.1.0-alpha.1",
      gitCommit: commit,
      generatedAt: "2026-07-13T00:00:00Z",
      supportMode: "explicit_run_and_import",
      runner: { os: "macOS", arch: "arm64" },
      fixtureContractSha256: createHash("sha256")
        .update(readFileSync(new URL("../../fixtures/compatibility/live-workflow-contract.json", import.meta.url)))
        .digest("hex"),
      scenarioMatrixSha256: createHash("sha256")
        .update(readFileSync(new URL("../../fixtures/compatibility/scenarios.json", import.meta.url)))
        .digest("hex"),
      previouslyBinarySha256: "3".repeat(64),
      localMappedRegression: {
        status: "separate_artifact_passed",
        artifactSha256: mappedSha,
        gitCommit: commit,
        scenarioCount: 30,
        evidencePath: "live-compatibility.json.evidence/mapped-compatibility-results.json",
      },
      codexCli: {
        runs: [
          { role: "latest", version: "2.0.0", binarySha256: "4".repeat(64), scenarios: structuredClone(scenarios) },
          { role: "previous", version: "1.9.0", binarySha256: "5".repeat(64), scenarios: structuredClone(scenarios) },
        ],
      },
      codexApp: {
        current: { status: "passed", build: "current", evidenceSha256: "6".repeat(64) },
        previous: { status: "unavailable", reason: "not obtainable", checkedAt: "2026-07-13T00:00:00Z" },
      },
      liveCodexWorkflowMatrix: { status: "complete", completedRuns: 60 },
      transparentCaptureReleaseGate: { eligible: true },
      releaseEligibility: { eligible: true, dataLossEvents: 0, seriousStaleApplications: 0 },
    };
    for (const run of artifact.codexCli.runs) {
      for (const scenario of run.scenarios) {
        const evidenceDirectory = join(evidenceRoot, run.role, scenario.id);
        mkdirSync(evidenceDirectory, { recursive: true });
        const evidence = {
          schemaVersion: 1,
          role: run.role,
          codexVersion: run.version,
          scenario: { id: scenario.id, category: scenario.category },
          scenarioAssertion: scenario.scenarioAssertion,
          verification: {
            passed: true,
            reconstruction: scenario.reconstruction,
            observed: { sourceIds: [`src-${"1".repeat(64)}`] },
          },
          groundTruth: { initialContentSha256: "2".repeat(64) },
        };
        const bytes = `${JSON.stringify(evidence)}\n`;
        const evidencePath = join(evidenceDirectory, "evidence.json");
        writeFileSync(evidencePath, bytes);
        scenario.evidenceSha256 = createHash("sha256").update(bytes).digest("hex");
        scenario.evidencePath = `live-compatibility.json.evidence/${run.role}/${scenario.id}/evidence.json`;
      }
    }
    const path = join(directory, "live-compatibility.json");
    writeFileSync(path, `${JSON.stringify(artifact)}\n`);
    const validator = fileURLToPath(new URL("../validate-live-compatibility.mjs", import.meta.url));
    const accepted = spawnSync(process.execPath, [validator, path, "--commit", commit, "--product-version", "0.1.0-alpha.1"], {
      encoding: "utf8",
    });
    assert.equal(accepted.status, 0, accepted.stderr);
    artifact.codexCli.runs[1].scenarios[0].scenarioAssertion.testFilter = "wrong-filter";
    writeFileSync(path, `${JSON.stringify(artifact)}\n`);
    const rejected = spawnSync(process.execPath, [validator, path, "--commit", commit, "--product-version", "0.1.0-alpha.1"], {
      encoding: "utf8",
    });
    assert.notEqual(rejected.status, 0);
    assert.match(rejected.stderr, /scenarioAssertion\.testFilter/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
