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
  finalizeCompletedArtifact,
  packageEvidenceBundle,
  prepareResumeArtifact,
  requireLiveArguments,
  validateMappedArtifact,
  validateFixtureContract,
  verifyScenarioEvidence,
} from "./live-harness.mjs";

const matrix = JSON.parse(readFileSync(new URL("../../fixtures/compatibility/scenarios.json", import.meta.url), "utf8"));
const contract = JSON.parse(readFileSync(new URL("../../fixtures/compatibility/live-workflow-contract.json", import.meta.url), "utf8"));

function createFinalizeFixture() {
  const directory = mkdtempSync(join(tmpdir(), "previously-live-finalize-test-"));
  const output = join(directory, "live-compatibility.json");
  const evidenceRoot = join(directory, "live-compatibility.json.evidence");
  mkdirSync(evidenceRoot, { recursive: true });
  const mappedBytes = "retained mapped evidence\n";
  const mappedSha = createHash("sha256").update(mappedBytes).digest("hex");
  writeFileSync(join(evidenceRoot, "mapped-compatibility-results.json"), mappedBytes);
  const reconstruction = Object.fromEntries(contract.requiredReconstruction.map((field) => [field, true]));
  const runs = [
    { role: "latest", version: "2.0.0", binarySha256: "4".repeat(64), scenarios: [] },
    { role: "previous", version: "1.9.0", binarySha256: "5".repeat(64), scenarios: [] },
  ];
  for (const run of runs) {
    for (const scenario of matrix.scenarios) {
      const scenarioAssertion = {
        status: "passed",
        testTarget: scenario.testTarget,
        testFilter: scenario.testFilter,
        expectation: scenario.expectation,
        mappedArtifactSha256: mappedSha,
      };
      const evidence = {
        schemaVersion: 1,
        role: run.role,
        codexVersion: run.version,
        codexExecution: contract.execution,
        scenario: { id: scenario.id, category: scenario.category },
        scenarioAssertion,
        verification: { passed: true, reconstruction },
      };
      const bytes = `${JSON.stringify(evidence)}\n`;
      const path = join(evidenceRoot, run.role, scenario.id, "evidence.json");
      mkdirSync(join(evidenceRoot, run.role, scenario.id), { recursive: true });
      writeFileSync(path, bytes);
      run.scenarios.push({
        id: scenario.id,
        category: scenario.category,
        status: "passed",
        codexExecution: contract.execution,
        reconstruction,
        scenarioAssertion,
        dataLossEvents: 0,
        evidenceSha256: createHash("sha256").update(bytes).digest("hex"),
        evidencePath: `live-compatibility.json.evidence/${run.role}/${scenario.id}/evidence.json`,
      });
    }
  }
  const appEvidence = {
    schemaVersion: 1,
    evidenceClass: "codex_app_verification",
    product: "Codex App",
    status: "passed",
    build: "current-build",
    checkedAt: "2026-07-13T00:00:00Z",
  };
  const appBytes = `${JSON.stringify(appEvidence)}\n`;
  const appPath = join(evidenceRoot, "codex-app", "current.json");
  mkdirSync(join(evidenceRoot, "codex-app"), { recursive: true });
  writeFileSync(appPath, appBytes);
  const codexApp = {
    current: {
      status: "passed",
      build: "current-build",
      evidenceSha256: createHash("sha256").update(appBytes).digest("hex"),
      evidencePath: "live-compatibility.json.evidence/codex-app/current.json",
    },
    previous: { status: "unavailable", reason: "not obtainable", checkedAt: "2026-07-13T00:00:00Z" },
  };
  const categories = Object.fromEntries(
    Object.entries(matrix.scenarios.reduce((counts, scenario) => {
      counts[scenario.category] = (counts[scenario.category] ?? 0) + 1;
      return counts;
    }, {})),
  );
  const expected = {
    productVersion: "0.1.0-alpha.1",
    gitCommit: "d".repeat(40),
    runner: { os: "macOS", arch: "arm64" },
    codexExecution: contract.execution,
    categories,
    fixtureContractSha256: "1".repeat(64),
    scenarioMatrixSha256: "2".repeat(64),
    previouslyBinarySha256: "3".repeat(64),
    codexRuns: runs.map(({ role, version, binarySha256 }) => ({ role, version, binarySha256, scenarios: [] })),
    codexApp,
    mappedArtifactSha256: mappedSha,
    scenarioIds: matrix.scenarios.map(({ id }) => id),
    scenarios: matrix.scenarios,
  };
  const artifact = {
    schemaVersion: 1,
    evidenceClass: "live_codex_workflow_matrix",
    product: "PreviouslyOn",
    productVersion: expected.productVersion,
    gitCommit: expected.gitCommit,
    generatedAt: "2026-07-13T00:00:00Z",
    supportMode: "explicit_run_and_import",
    runner: expected.runner,
    codexExecution: expected.codexExecution,
    categories,
    fixtureContractSha256: expected.fixtureContractSha256,
    scenarioMatrixSha256: expected.scenarioMatrixSha256,
    previouslyBinarySha256: expected.previouslyBinarySha256,
    codexCli: { runs },
    codexApp,
    localMappedRegression: { artifactSha256: mappedSha, gitCommit: expected.gitCommit },
    liveCodexWorkflowMatrix: { status: "complete", requiredRuns: 60, completedRuns: 60 },
    transparentCaptureReleaseGate: { eligible: false, reason: "serious stale applications remain unmeasured" },
    releaseEligibility: {
      eligible: false,
      dataLossEvents: 0,
      seriousStaleApplications: { status: "unmeasured", reason: "not evaluated" },
    },
  };
  const staleEvidence = {
    schemaVersion: 1,
    evidenceClass: "serious_stale_application_evaluation",
    product: "PreviouslyOn",
    productVersion: expected.productVersion,
    gitCommit: expected.gitCommit,
    status: "measured",
    scenariosEvaluated: 60,
    seriousStaleApplications: 0,
    evaluatedAt: "2026-07-13T01:00:00Z",
  };
  const staleBytes = `${JSON.stringify(staleEvidence)}\n`;
  const stalePath = join(evidenceRoot, "evaluations", "serious-stale-applications.json");
  mkdirSync(join(evidenceRoot, "evaluations"), { recursive: true });
  writeFileSync(stalePath, staleBytes);
  const measured = {
    status: "measured",
    count: 0,
    scenariosEvaluated: 60,
    evaluatedAt: staleEvidence.evaluatedAt,
    evidenceSha256: createHash("sha256").update(staleBytes).digest("hex"),
    evidencePath: "live-compatibility.json.evidence/evaluations/serious-stale-applications.json",
  };
  return { directory, output, evidenceRoot, expected, artifact, measured, stalePath, staleBytes };
}

test("live fixture contract expands to 60 two-turn workflow slots without executing sessions", () => {
  const validated = validateFixtureContract(matrix, contract);
  assert.equal(validated.scenarios.length, 30);
  assert.deepEqual(Object.values(validated.categories).sort(), [6, 6, 6, 6, 6]);
  const plan = buildDryRunPlan(matrix, contract);
  assert.equal(plan.requiredRuns, 60);
  assert.equal(plan.paidSessionsExecuted, 0);
  assert.deepEqual(plan.codexExecution, contract.execution);
  assert.equal(plan.transparentCaptureReleaseGate.eligible, false);
  assert.equal(plan.workflows.length, 30);
  assert.match(plan.workflows[0].initialPrompt, /apply_patch/);
  assert.match(plan.workflows[0].resumePrompt, /verify\.sh/);
  const invalidExecution = structuredClone(contract);
  invalidExecution.execution.sandbox = "danger-full-access";
  assert.throws(
    () => validateFixtureContract(matrix, invalidExecution),
    /bind a valid Codex model and reasoning effort/,
  );
});

test("normal live and resume modes reject pre-attached stale evaluation evidence", () => {
  for (const resume of [false, true]) {
    assert.throws(
      () => requireLiveArguments(
        {
          confirm: contract.confirmationPhrase,
          resume,
          staleEvaluationArtifact: "/tmp/stale-evaluation.json",
          staleEvaluationSha256: "a".repeat(64),
        },
        contract,
      ),
      /finalize-only; live run\/resume must remain unmeasured/,
    );
  }
});

test("live workflow markers never collide with the secret redaction corpus", () => {
  const scenario = matrix.scenarios.find(({ id }) => id === "privacy-secret-corpus");
  const fixture = buildWorkflowFixture(scenario, contract);
  assert.equal(fixture.id, "privacy-secret-corpus");
  assert.equal(fixture.runtimeSlug, "privacy-redacted-corpus");
  assert.doesNotMatch(fixture.runtimeSlug, /secret/i);
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
    event("user_prompt", { prompt: fixture.initialPrompt, model: contract.execution.model }),
    ...filePair(fixture.INITIAL_VALUE, "tool-initial"),
    event("tool_finished", { tool_use_id: "test-initial", tool_input: { command: fixture.initialTestCommand }, tool_response: { exit_code: 0 } }),
    event(
      "assistant_final",
      { last_assistant_message: fixture.INITIAL_FINAL },
      `codex-app-server:thread:${sessionId}:item:final-initial:assistant-final`,
    ),
    event("session_stopped", { last_assistant_message: fixture.INITIAL_FINAL }),
    event("user_prompt", { prompt: fixture.resumePrompt, model: contract.execution.model }),
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
    codexExecution: contract.execution,
  });
  assert.equal(verified.passed, true);
  assert.deepEqual(verified.reconstruction, {
    userPrompt: true,
    assistantFinal: true,
    fileChangeTool: true,
    testCommand: true,
    modelIdentity: true,
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
      codexExecution: contract.execution,
    }).reconstruction.stableSourceIds,
    false,
  );
  events[0].coverage.missing.length = 0;
  events[0].payload.model = "gpt-5.6-terra";
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
      codexExecution: contract.execution,
    }).reconstruction.modelIdentity,
    false,
  );
});

test("release eligibility is true only for two complete distinct 30-scenario runs", () => {
  const ids = matrix.scenarios.map((scenario) => scenario.id);
  const scenarioResults = matrix.scenarios.map((scenario) => ({
    id: scenario.id,
    status: "passed",
    codexExecution: contract.execution,
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
    codexExecution: contract.execution,
    codexCli: {
      runs: [
        { role: "latest", version: "2.0.0", scenarios: structuredClone(scenarioResults) },
        { role: "previous", version: "1.9.0", scenarios: structuredClone(scenarioResults) },
      ],
    },
    releaseEligibility: {
      dataLossEvents: 0,
      seriousStaleApplications: {
        status: "measured",
        count: 0,
        evidenceSha256: "d".repeat(64),
        evidencePath: "evidence/stale.json",
      },
    },
    codexApp: {
      current: {
        status: "passed",
        build: "current-build",
        evidenceSha256: "b".repeat(64),
        evidencePath: "evidence/current-app.json",
      },
      previous: { status: "unavailable", reason: "vendor build unavailable", checkedAt: "2026-07-13T00:00:00Z" },
    },
  };
  assert.equal(computeEligibility(artifact, ids), true);
  artifact.releaseEligibility.seriousStaleApplications = { status: "unmeasured" };
  assert.equal(computeEligibility(artifact, ids), false);
  artifact.releaseEligibility.seriousStaleApplications = {
    status: "measured",
    count: 0,
    evidenceSha256: "d".repeat(64),
    evidencePath: "evidence/stale.json",
  };
  artifact.codexCli.runs[1].scenarios[12].reconstruction.testCommand = false;
  assert.equal(computeEligibility(artifact, ids), false);
});

test("resume validates immutable bindings and skips only intact passed checkpoints", () => {
  const directory = mkdtempSync(join(tmpdir(), "previously-live-resume-test-"));
  try {
    const output = join(directory, "live-compatibility.json");
    const evidenceRoot = join(directory, "live-compatibility.json.evidence");
    const mappedBytes = "mapped evidence\n";
    const mappedSha = createHash("sha256").update(mappedBytes).digest("hex");
    mkdirSync(evidenceRoot, { recursive: true });
    writeFileSync(join(evidenceRoot, "mapped-compatibility-results.json"), mappedBytes);
    const scenario = matrix.scenarios[0];
    const assertion = {
      status: "passed",
      testTarget: scenario.testTarget,
      testFilter: scenario.testFilter,
      expectation: scenario.expectation,
      mappedArtifactSha256: mappedSha,
    };
    const reconstruction = Object.fromEntries(contract.requiredReconstruction.map((field) => [field, true]));
    const evidence = {
      schemaVersion: 1,
      role: "latest",
      codexVersion: "2.0.0",
      codexExecution: contract.execution,
      scenario: { id: scenario.id, category: scenario.category },
      scenarioAssertion: assertion,
      verification: { passed: true, reconstruction },
    };
    const evidenceBytes = `${JSON.stringify(evidence)}\n`;
    const evidencePath = join(evidenceRoot, "latest", scenario.id, "evidence.json");
    mkdirSync(join(evidenceRoot, "latest", scenario.id), { recursive: true });
    writeFileSync(evidencePath, evidenceBytes);
    const appEvidence = {
      schemaVersion: 1,
      evidenceClass: "codex_app_verification",
      product: "Codex App",
      status: "passed",
      build: "current",
      checkedAt: "2026-07-13T00:00:00Z",
    };
    const appEvidenceBytes = `${JSON.stringify(appEvidence)}\n`;
    const appEvidencePath = join(evidenceRoot, "codex-app", "current.json");
    mkdirSync(join(evidenceRoot, "codex-app"), { recursive: true });
    writeFileSync(appEvidencePath, appEvidenceBytes);
    const codexApp = {
      current: {
        status: "passed",
        build: "current",
        evidenceSha256: createHash("sha256").update(appEvidenceBytes).digest("hex"),
        evidencePath: "live-compatibility.json.evidence/codex-app/current.json",
      },
      previous: { status: "unavailable", reason: "not obtainable", checkedAt: "2026-07-13T00:00:00Z" },
    };
    const seriousStaleApplications = { status: "unmeasured", reason: "not evaluated" };
    const codexRuns = [
      { role: "latest", version: "2.0.0", binarySha256: "4".repeat(64), scenarios: [] },
      { role: "previous", version: "1.9.0", binarySha256: "5".repeat(64), scenarios: [] },
    ];
    const expected = {
      productVersion: "0.1.0-alpha.1",
      gitCommit: "d".repeat(40),
      runner: { os: "macOS", arch: "arm64" },
      codexExecution: contract.execution,
      categories: Object.fromEntries(
        Object.entries(matrix.scenarios.reduce((counts, scenario) => {
          counts[scenario.category] = (counts[scenario.category] ?? 0) + 1;
          return counts;
        }, {})),
      ),
      fixtureContractSha256: "1".repeat(64),
      scenarioMatrixSha256: "2".repeat(64),
      previouslyBinarySha256: "3".repeat(64),
      codexRuns,
      codexApp,
      seriousStaleApplications,
      mappedArtifactSha256: mappedSha,
      scenarioIds: matrix.scenarios.map(({ id }) => id),
      scenarios: matrix.scenarios,
    };
    const passed = {
      id: scenario.id,
      category: scenario.category,
      status: "passed",
      codexExecution: contract.execution,
      reconstruction,
      scenarioAssertion: assertion,
      dataLossEvents: 0,
      evidenceSha256: createHash("sha256").update(evidenceBytes).digest("hex"),
      evidencePath: `live-compatibility.json.evidence/latest/${scenario.id}/evidence.json`,
    };
    const artifact = {
      schemaVersion: 1,
      evidenceClass: "live_codex_workflow_matrix",
      product: "PreviouslyOn",
      productVersion: expected.productVersion,
      gitCommit: expected.gitCommit,
      runner: expected.runner,
      codexExecution: expected.codexExecution,
      categories: expected.categories,
      supportMode: "explicit_run_and_import",
      fixtureContractSha256: expected.fixtureContractSha256,
      scenarioMatrixSha256: expected.scenarioMatrixSha256,
      previouslyBinarySha256: expected.previouslyBinarySha256,
      codexApp,
      localMappedRegression: { artifactSha256: mappedSha, gitCommit: expected.gitCommit },
      codexCli: {
        runs: [
          { ...codexRuns[0], scenarios: [passed, { id: matrix.scenarios[1].id, status: "failed" }] },
          { ...codexRuns[1], scenarios: [] },
        ],
      },
      liveCodexWorkflowMatrix: { status: "failed", requiredRuns: 60, completedRuns: 999 },
      transparentCaptureReleaseGate: { eligible: false },
      releaseEligibility: { eligible: false, dataLossEvents: 999, seriousStaleApplications },
    };
    const resumed = prepareResumeArtifact(structuredClone(artifact), expected, output, evidenceRoot);
    assert.equal(resumed.liveCodexWorkflowMatrix.completedRuns, 1);
    assert.equal(resumed.codexCli.runs[0].scenarios.length, 1);
    assert.equal(resumed.releaseEligibility.dataLossEvents, 0);
    assert.equal(resumed.liveCodexWorkflowMatrix.status, "running");

    const changedBinary = structuredClone(expected);
    changedBinary.previouslyBinarySha256 = "9".repeat(64);
    assert.throws(
      () => prepareResumeArtifact(structuredClone(artifact), changedBinary, output, evidenceRoot),
      /does not match the current commit, product binary, or fixture bytes/,
    );
    const changedExecution = structuredClone(expected);
    changedExecution.codexExecution = { ...contract.execution, model: "gpt-5.6-terra" };
    assert.throws(
      () => prepareResumeArtifact(structuredClone(artifact), changedExecution, output, evidenceRoot),
      /does not match the current commit, product binary, or fixture bytes/,
    );
    writeFileSync(appEvidencePath, "tampered\n");
    assert.throws(
      () => prepareResumeArtifact(structuredClone(artifact), expected, output, evidenceRoot),
      /retained file hash changed/,
    );
    writeFileSync(appEvidencePath, appEvidenceBytes);
    writeFileSync(evidencePath, "tampered\n");
    assert.throws(
      () => prepareResumeArtifact(structuredClone(artifact), expected, output, evidenceRoot),
      /missing or has changed/,
    );
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("stale-evaluation finalize revalidates all 60 retained workflows without rerunning them", () => {
  const fixture = createFinalizeFixture();
  try {
    const retainedBefore = fixture.artifact.codexCli.runs.flatMap((run) =>
      run.scenarios.map((scenario) => [scenario.evidencePath, scenario.evidenceSha256]),
    );
    const finalized = finalizeCompletedArtifact(
      structuredClone(fixture.artifact),
      fixture.expected,
      fixture.output,
      fixture.evidenceRoot,
      fixture.measured,
    );
    assert.equal(finalized.liveCodexWorkflowMatrix.status, "complete");
    assert.equal(finalized.liveCodexWorkflowMatrix.completedRuns, 60);
    assert.equal(finalized.releaseEligibility.seriousStaleApplications.status, "measured");
    assert.equal(finalized.releaseEligibility.eligible, true);
    assert.equal(finalized.transparentCaptureReleaseGate.eligible, true);
    assert.deepEqual(
      finalized.codexCli.runs.flatMap((run) =>
        run.scenarios.map((scenario) => [scenario.evidencePath, scenario.evidenceSha256]),
      ),
      retainedBefore,
    );
    assert.equal(finalized.codexCli.runs.flatMap((run) => run.scenarios).length, 60);
  } finally {
    rmSync(fixture.directory, { recursive: true, force: true });
  }
});

test("stale-evaluation finalize rejects replacement, incomplete, modified, or rebound evidence", async (t) => {
  const fixture = createFinalizeFixture();
  try {
    await t.test("measured evidence cannot be replaced", () => {
      const artifact = structuredClone(fixture.artifact);
      artifact.releaseEligibility.seriousStaleApplications = fixture.measured;
      assert.throws(
        () => finalizeCompletedArtifact(artifact, fixture.expected, fixture.output, fixture.evidenceRoot, fixture.measured),
        /requires an unmeasured artifact/,
      );
    });
    await t.test("incomplete matrices and data loss are rejected", () => {
      const incomplete = structuredClone(fixture.artifact);
      incomplete.codexCli.runs[1].scenarios.pop();
      incomplete.liveCodexWorkflowMatrix.completedRuns = 59;
      assert.throws(
        () => finalizeCompletedArtifact(incomplete, fixture.expected, fixture.output, fixture.evidenceRoot, fixture.measured),
        /complete, ineligible, lossless 60-run artifact/,
      );
      const lossy = structuredClone(fixture.artifact);
      lossy.releaseEligibility.dataLossEvents = 1;
      assert.throws(
        () => finalizeCompletedArtifact(lossy, fixture.expected, fixture.output, fixture.evidenceRoot, fixture.measured),
        /complete, ineligible, lossless 60-run artifact/,
      );
    });
    await t.test("modified scenario evidence is rejected", () => {
      const scenario = fixture.artifact.codexCli.runs[0].scenarios[0];
      const path = join(fixture.directory, scenario.evidencePath);
      const original = readFileSync(path);
      writeFileSync(path, "tampered\n");
      assert.throws(
        () => finalizeCompletedArtifact(structuredClone(fixture.artifact), fixture.expected, fixture.output, fixture.evidenceRoot, fixture.measured),
        /missing or has changed/,
      );
      writeFileSync(path, original);
    });
    await t.test("App and immutable binary bindings cannot change", () => {
      const changedApp = structuredClone(fixture.expected);
      changedApp.codexApp.current.build = "different-build";
      assert.throws(
        () => finalizeCompletedArtifact(structuredClone(fixture.artifact), changedApp, fixture.output, fixture.evidenceRoot, fixture.measured),
        /does not match the supplied Codex App evidence/,
      );
      const changedBinary = structuredClone(fixture.expected);
      changedBinary.previouslyBinarySha256 = "9".repeat(64);
      assert.throws(
        () => finalizeCompletedArtifact(structuredClone(fixture.artifact), changedBinary, fixture.output, fixture.evidenceRoot, fixture.measured),
        /does not match the current commit, product binary, or fixture bytes/,
      );
    });
    await t.test("evaluator must cover 60 workflows and retain its exact digest", () => {
      assert.throws(
        () => finalizeCompletedArtifact(
          structuredClone(fixture.artifact),
          fixture.expected,
          fixture.output,
          fixture.evidenceRoot,
          { ...fixture.measured, scenariosEvaluated: 59 },
        ),
        /covering all 60 workflows/,
      );
      writeFileSync(fixture.stalePath, "tampered\n");
      assert.throws(
        () => finalizeCompletedArtifact(structuredClone(fixture.artifact), fixture.expected, fixture.output, fixture.evidenceRoot, fixture.measured),
        /retained file hash changed/,
      );
      writeFileSync(fixture.stalePath, fixture.staleBytes);
    });
  } finally {
    rmSync(fixture.directory, { recursive: true, force: true });
  }
});

test("mapped artifact validation binds all scenario assertions to the same commit and CLI versions", () => {
  const directory = mkdtempSync(join(tmpdir(), "previously-mapped-artifact-test-"));
  try {
    const matrixBytes = readFileSync(new URL("../../fixtures/compatibility/scenarios.json", import.meta.url));
    const ids = matrix.scenarios.map((scenario) => scenario.id);
    const actualRows = matrix.scenarios.map((scenario) => ({
      id: scenario.id,
      testTarget: scenario.testTarget,
      testFilter: scenario.testFilter,
      expectation: scenario.expectation,
      status: "passed",
      exitCode: 0,
    }));
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
          { codexVersionSlot: "2.0.0", scenarioResults: structuredClone(actualRows), passed: ids, failed: [] },
          { codexVersionSlot: "1.9.0", scenarioResults: structuredClone(actualRows), passed: ids, failed: [] },
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
    artifact.localMappedRegression.runs[1].scenarioResults[0].testFilter = "misspelled-filter";
    writeFileSync(path, `${JSON.stringify(artifact)}\n`);
    assert.throws(
      () => validateMappedArtifact(path, matrix.scenarios, "d".repeat(40), "0.1.0-alpha.1", "2.0.0", "1.9.0"),
      /invalid actual row outcome/,
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
    const actualRows = matrix.scenarios.map((scenario) => ({
      id: scenario.id,
      testTarget: scenario.testTarget,
      testFilter: scenario.testFilter,
      expectation: scenario.expectation,
      status: "passed",
      exitCode: 0,
    }));
    const mappedEvidence = {
      schemaVersion: 1,
      gitCommit: commit,
      localMappedRegression: {
        scenarioCount: 30,
        runs: [
          { codexVersionSlot: "2.0.0", scenarioResults: structuredClone(actualRows) },
          { codexVersionSlot: "1.9.0", scenarioResults: structuredClone(actualRows) },
        ],
      },
    };
    const mappedBytes = `${JSON.stringify(mappedEvidence)}\n`;
    const mappedPath = join(evidenceRoot, "mapped-compatibility-results.json");
    writeFileSync(mappedPath, mappedBytes);
    const mappedSha = createHash("sha256").update(mappedBytes).digest("hex");
    const scenarios = matrix.scenarios.map((scenario) => ({
      id: scenario.id,
      category: scenario.category,
      status: "passed",
      codexExecution: contract.execution,
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
      codexExecution: contract.execution,
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
      codexApp: {},
      liveCodexWorkflowMatrix: { status: "complete", completedRuns: 60 },
      transparentCaptureReleaseGate: { eligible: true },
      releaseEligibility: { eligible: true, dataLossEvents: 0 },
    };
    const appEvidence = {
      schemaVersion: 1,
      evidenceClass: "codex_app_verification",
      product: "Codex App",
      status: "passed",
      build: "current",
      checkedAt: "2026-07-13T00:00:00Z",
    };
    const appBytes = `${JSON.stringify(appEvidence)}\n`;
    const appPath = join(evidenceRoot, "codex-app", "current.json");
    mkdirSync(join(evidenceRoot, "codex-app"), { recursive: true });
    writeFileSync(appPath, appBytes);
    artifact.codexApp = {
      current: {
        status: "passed",
        build: "current",
        evidenceSha256: createHash("sha256").update(appBytes).digest("hex"),
        evidencePath: "live-compatibility.json.evidence/codex-app/current.json",
      },
      previous: { status: "unavailable", reason: "not obtainable", checkedAt: "2026-07-13T00:00:00Z" },
    };
    const staleEvidence = {
      schemaVersion: 1,
      evidenceClass: "serious_stale_application_evaluation",
      product: "PreviouslyOn",
      productVersion: "0.1.0-alpha.1",
      gitCommit: commit,
      status: "measured",
      scenariosEvaluated: 60,
      seriousStaleApplications: 0,
      evaluatedAt: "2026-07-13T00:00:00Z",
    };
    const staleBytes = `${JSON.stringify(staleEvidence)}\n`;
    const stalePath = join(evidenceRoot, "evaluations", "serious-stale-applications.json");
    mkdirSync(join(evidenceRoot, "evaluations"), { recursive: true });
    writeFileSync(stalePath, staleBytes);
    artifact.releaseEligibility.seriousStaleApplications = {
      status: "measured",
      count: 0,
      scenariosEvaluated: 60,
      evaluatedAt: "2026-07-13T00:00:00Z",
      evidenceSha256: createHash("sha256").update(staleBytes).digest("hex"),
      evidencePath: "live-compatibility.json.evidence/evaluations/serious-stale-applications.json",
    };
    for (const run of artifact.codexCli.runs) {
      for (const scenario of run.scenarios) {
        const evidenceDirectory = join(evidenceRoot, run.role, scenario.id);
        mkdirSync(evidenceDirectory, { recursive: true });
        const evidence = {
          schemaVersion: 1,
          role: run.role,
          codexVersion: run.version,
          codexExecution: contract.execution,
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
    artifact.codexExecution = { ...contract.execution, model: "gpt-5.6-terra" };
    writeFileSync(path, `${JSON.stringify(artifact)}\n`);
    const wrongModel = spawnSync(process.execPath, [validator, path, "--commit", commit, "--product-version", "0.1.0-alpha.1"], {
      encoding: "utf8",
    });
    assert.notEqual(wrongModel.status, 0);
    assert.match(wrongModel.stderr, /codexExecution\.model/);
    artifact.codexExecution = contract.execution;
    artifact.codexCli.runs[0].scenarios[0].codexExecution = {
      ...contract.execution,
      timeoutSeconds: contract.execution.timeoutSeconds + 1,
    };
    writeFileSync(path, `${JSON.stringify(artifact)}\n`);
    const wrongScenarioExecution = spawnSync(
      process.execPath,
      [validator, path, "--commit", commit, "--product-version", "0.1.0-alpha.1"],
      { encoding: "utf8" },
    );
    assert.notEqual(wrongScenarioExecution.status, 0);
    assert.match(wrongScenarioExecution.stderr, /codexExecution\.timeoutSeconds/);
    artifact.codexCli.runs[0].scenarios[0].codexExecution = contract.execution;
    const measured = artifact.releaseEligibility.seriousStaleApplications;
    artifact.releaseEligibility.seriousStaleApplications = 0;
    writeFileSync(path, `${JSON.stringify(artifact)}\n`);
    const hardcoded = spawnSync(process.execPath, [validator, path, "--commit", commit, "--product-version", "0.1.0-alpha.1"], {
      encoding: "utf8",
    });
    assert.notEqual(hardcoded.status, 0);
    assert.match(hardcoded.stderr, /seriousStaleApplications\.status/);
    artifact.releaseEligibility.seriousStaleApplications = measured;
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
