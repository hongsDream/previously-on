#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import {
  accessSync,
  chmodSync,
  closeSync,
  copyFileSync,
  existsSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  renameSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { constants as fsConstants } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve } from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

const SCRIPT_PATH = fileURLToPath(import.meta.url);
const ROOT = resolve(dirname(SCRIPT_PATH), "../..");
const MATRIX_PATH = join(ROOT, "fixtures/compatibility/scenarios.json");
const CONTRACT_PATH = join(ROOT, "fixtures/compatibility/live-workflow-contract.json");
const REQUIRED_RECONSTRUCTION = [
  "userPrompt",
  "assistantFinal",
  "fileChangeTool",
  "testCommand",
  "modelIdentity",
  "stableSourceIds",
];
const MAX_CAPTURE_BYTES = 32 * 1024 * 1024;

export function validateFixtureContract(matrix, contract) {
  if (matrix?.schemaVersion !== 1 || contract?.schemaVersion !== 1) {
    throw new Error("compatibility fixtures must use schemaVersion 1");
  }
  const scenarios = matrix.scenarios;
  if (!Array.isArray(scenarios) || scenarios.length !== 30) {
    throw new Error("live compatibility requires exactly 30 scenario definitions");
  }
  const ids = new Set(scenarios.map((scenario) => scenario.id));
  if (ids.size !== 30 || [...ids].some((id) => !/^[a-z0-9-]+$/.test(id))) {
    throw new Error("scenario IDs must be 30 unique lowercase slugs");
  }
  const categories = new Map();
  for (const scenario of scenarios) {
    if (
      typeof scenario.testTarget !== "string" ||
      typeof scenario.testFilter !== "string" ||
      typeof scenario.expectation !== "string" ||
      !scenario.testTarget ||
      !scenario.testFilter ||
      !scenario.expectation
    ) {
      throw new Error(`scenario ${scenario.id} omitted its mapped assertion contract`);
    }
    categories.set(scenario.category, (categories.get(scenario.category) ?? 0) + 1);
  }
  if (categories.size !== 5 || [...categories.values()].some((count) => count !== 6)) {
    throw new Error("live compatibility requires five categories with six scenarios each");
  }
  if (contract.workflowCountPerVersion !== 30 || contract.turnsPerWorkflow !== 2) {
    throw new Error("live contract must declare 30 two-turn workflows per version");
  }
  if (contract.confirmationPhrase !== "RUN_60_AUTHENTICATED_CODEX_WORKFLOWS") {
    throw new Error("live contract confirmation phrase changed unexpectedly");
  }
  if (
    typeof contract.execution?.model !== "string" ||
    !/^gpt-[a-z0-9.-]+$/.test(contract.execution.model) ||
    !["low", "medium", "high", "xhigh"].includes(contract.execution?.reasoningEffort) ||
    contract.execution?.sandbox !== "workspace-write" ||
    contract.execution?.strictConfig !== true ||
    !Number.isInteger(contract.execution?.timeoutSeconds) ||
    contract.execution.timeoutSeconds < 30
  ) {
    throw new Error("live contract must bind a valid Codex model and reasoning effort");
  }
  for (const field of REQUIRED_RECONSTRUCTION) {
    if (!contract.requiredReconstruction?.includes(field)) {
      throw new Error(`live contract omitted reconstruction field ${field}`);
    }
  }
  for (const prompt of [contract.prompts?.initial, contract.prompts?.resume]) {
    if (typeof prompt !== "string" || !prompt.includes("{{SCENARIO_ID}}")) {
      throw new Error("prompt templates must contain {{SCENARIO_ID}}");
    }
  }
  return { scenarios, categories: Object.fromEntries(categories) };
}

export function buildWorkflowFixture(scenario, contract) {
  const workflowMarker = scenario.id.replace(/secret/gi, "redacted");
  const replacements = {
    SCENARIO_ID: workflowMarker,
    INITIAL_VALUE: render(contract.fixture.initialValueTemplate, { SCENARIO_ID: workflowMarker }),
    RESUME_VALUE: render(contract.fixture.resumeValueTemplate, { SCENARIO_ID: workflowMarker }),
    INITIAL_FINAL: render(contract.fixture.initialFinalTemplate, { SCENARIO_ID: workflowMarker }),
    RESUME_FINAL: render(contract.fixture.resumeFinalTemplate, { SCENARIO_ID: workflowMarker }),
  };
  return {
    id: scenario.id,
    runtimeSlug: workflowMarker,
    category: scenario.category,
    testTarget: scenario.testTarget,
    testFilter: scenario.testFilter,
    expectation: scenario.expectation,
    ...replacements,
    initialPrompt: render(contract.prompts.initial, replacements),
    resumePrompt: render(contract.prompts.resume, replacements),
    initialTestCommand: `./${contract.fixture.verifyScript} ${replacements.INITIAL_VALUE}`,
    resumeTestCommand: `./${contract.fixture.verifyScript} ${replacements.RESUME_VALUE}`,
  };
}

export function buildDryRunPlan(matrix, contract) {
  const { scenarios, categories } = validateFixtureContract(matrix, contract);
  const workflows = scenarios.map((scenario) => buildWorkflowFixture(scenario, contract));
  return {
    schemaVersion: 1,
    dryRun: true,
    evidenceClass: "live_codex_workflow_plan_only",
    paidSessionsExecuted: 0,
    requiredRuns: 60,
    workflowCountPerVersion: workflows.length,
    categories,
    codexExecution: contract.execution,
    versionRoles: ["latest", "previous"],
    commandsPerWorkflow: ["previously setup codex", "previously run codex -- exec", "previously run codex -- exec resume", "previously import codex", "previously export", "previously uninstall codex"],
    requiredReconstruction: REQUIRED_RECONSTRUCTION,
    supplementalReleaseEvidence: {
      mappedRegression: "a separate clean-commit mapped regression artifact covering each scenario is required",
      codexAppCurrent: "build identifier and sanitized evidence SHA-256 are required for live release eligibility",
      codexAppPrevious: "build and evidence SHA-256, or a documented unavailable reason, are required",
    },
    workflows,
    transparentCaptureReleaseGate: {
      eligible: false,
      reason: "dry-run validates schema and commands only; no authenticated Codex workflow ran",
    },
  };
}

export function verifyScenarioEvidence(input) {
  const {
    fixture,
    exportData,
    sessionId,
    initialFinalText,
    resumeFinalText,
    initialContent,
    resumeContent,
    gitStatus,
    codexExecution,
  } = input;
  const events = Array.isArray(exportData?.canonical_events) ? exportData.canonical_events : [];
  const promptInitial = findEvent(events, "user_prompt", fixture.INITIAL_VALUE, sessionId);
  const promptResume = findEvent(events, "user_prompt", fixture.RESUME_VALUE, sessionId);
  const finalInitial = findAssistantFinal(events, fixture.INITIAL_FINAL, sessionId);
  const finalResume = findAssistantFinal(events, fixture.RESUME_FINAL, sessionId);
  const fileInitial = findToolPair(events, sessionId, "state.txt", fixture.INITIAL_VALUE);
  const fileResume = findToolPair(events, sessionId, "state.txt", fixture.RESUME_VALUE);
  const testInitial = findEvent(events, "tool_finished", fixture.initialTestCommand, sessionId);
  const testResume = findEvent(events, "tool_finished", fixture.resumeTestCommand, sessionId);
  const requiredEvents = [
    promptInitial,
    promptResume,
    finalInitial,
    finalResume,
    fileInitial?.pre,
    fileInitial?.post,
    fileResume?.pre,
    fileResume?.post,
    testInitial,
    testResume,
  ];
  const stable = requiredEvents.every((event) => event && stableSource(event));
  const sourceIds = requiredEvents.filter(Boolean).map((event) => event.source_id);
  const uniqueSources = new Set(sourceIds).size === sourceIds.length;
  const filesystem =
    initialContent === `${fixture.INITIAL_VALUE}\n` &&
    resumeContent === `${fixture.RESUME_VALUE}\n` &&
    gitStatus.trim() === "M state.txt";
  const reconstruction = {
    userPrompt: Boolean(promptInitial && promptResume),
    assistantFinal: Boolean(
      finalInitial &&
        finalResume &&
        initialFinalText.includes(fixture.INITIAL_FINAL) &&
        resumeFinalText.includes(fixture.RESUME_FINAL),
    ),
    fileChangeTool: Boolean(fileInitial && fileResume),
    testCommand: Boolean(testInitial && testResume),
    modelIdentity:
      promptInitial?.payload?.model === codexExecution.model &&
      promptResume?.payload?.model === codexExecution.model,
    stableSourceIds: stable && uniqueSources,
  };
  return {
    reconstruction,
    filesystem,
    passed: filesystem && REQUIRED_RECONSTRUCTION.every((field) => reconstruction[field]),
    observed: {
      sessionId,
      sourceIds,
      fileToolUseIds: [fileInitial?.toolUseId, fileResume?.toolUseId].filter(Boolean),
      testSourceIds: [testInitial?.source_id, testResume?.source_id].filter(Boolean),
      canonicalEventCount: events.length,
    },
  };
}

export function computeEligibility(artifact, expectedScenarioIds) {
  if (!hasCompleteWorkflowMatrix(artifact, expectedScenarioIds)) return false;
  if (
    typeof artifact.codexExecution?.model !== "string" ||
    !["low", "medium", "high", "xhigh"].includes(artifact.codexExecution?.reasoningEffort) ||
    artifact.codexExecution?.sandbox !== "workspace-write" ||
    artifact.codexExecution?.strictConfig !== true ||
    !Number.isInteger(artifact.codexExecution?.timeoutSeconds) ||
    artifact.codexExecution.timeoutSeconds < 30
  ) {
    return false;
  }
  const currentApp = artifact.codexApp?.current;
  const previousApp = artifact.codexApp?.previous;
  if (
    currentApp?.status !== "passed" ||
    !currentApp.build ||
    !/^[0-9a-f]{64}$/.test(currentApp.evidenceSha256 ?? "") ||
    !currentApp.evidencePath
  ) {
    return false;
  }
  const previousValid =
    (previousApp?.status === "passed" &&
      previousApp.build &&
      /^[0-9a-f]{64}$/.test(previousApp.evidenceSha256 ?? "") &&
      previousApp.evidencePath) ||
    (previousApp?.status === "unavailable" && previousApp.reason && Number.isFinite(Date.parse(previousApp.checkedAt)));
  const stale = artifact.releaseEligibility?.seriousStaleApplications;
  return Boolean(previousValid) &&
    artifact.releaseEligibility?.dataLossEvents === 0 &&
    stale?.status === "measured" &&
    stale.count === 0 &&
    /^[0-9a-f]{64}$/.test(stale.evidenceSha256 ?? "") &&
    Boolean(stale.evidencePath);
}

export function hasCompleteWorkflowMatrix(artifact, expectedScenarioIds) {
  const runs = artifact.codexCli?.runs;
  if (!Array.isArray(runs) || runs.length !== 2) return false;
  const roles = new Map(runs.map((run) => [run.role, run]));
  if (roles.size !== 2 || !roles.has("latest") || !roles.has("previous")) return false;
  if (roles.get("latest").version === roles.get("previous").version) return false;
  for (const role of ["latest", "previous"]) {
    const scenarios = roles.get(role).scenarios;
    if (!Array.isArray(scenarios) || scenarios.length !== 30) return false;
    const ids = new Set(scenarios.map((scenario) => scenario.id));
    if (ids.size !== 30 || expectedScenarioIds.some((id) => !ids.has(id))) return false;
    for (const scenario of scenarios) {
      if (scenario.status !== "passed") return false;
      if (stableStringify(scenario.codexExecution) !== stableStringify(artifact.codexExecution)) return false;
      if (!REQUIRED_RECONSTRUCTION.every((field) => scenario.reconstruction?.[field] === true)) {
        return false;
      }
      if (
        scenario.scenarioAssertion?.status !== "passed" ||
        !scenario.scenarioAssertion.testTarget ||
        !scenario.scenarioAssertion.testFilter ||
        !scenario.scenarioAssertion.expectation ||
        !/^[0-9a-f]{64}$/.test(scenario.scenarioAssertion.mappedArtifactSha256 ?? "")
      ) {
        return false;
      }
      if (!/^[0-9a-f]{64}$/.test(scenario.evidenceSha256 ?? "")) return false;
    }
  }
  return true;
}

export function prepareResumeArtifact(artifact, expected, outputPath, evidenceRoot) {
  const identityMatches =
    artifact?.schemaVersion === 1 &&
    artifact?.evidenceClass === "live_codex_workflow_matrix" &&
    artifact?.product === "PreviouslyOn" &&
    artifact?.supportMode === "explicit_run_and_import" &&
    artifact?.productVersion === expected.productVersion &&
    artifact?.gitCommit === expected.gitCommit &&
    stableStringify(artifact?.runner) === stableStringify(expected.runner) &&
    stableStringify(artifact?.codexExecution) === stableStringify(expected.codexExecution) &&
    stableStringify(artifact?.categories) === stableStringify(expected.categories) &&
    artifact?.fixtureContractSha256 === expected.fixtureContractSha256 &&
    artifact?.scenarioMatrixSha256 === expected.scenarioMatrixSha256 &&
    artifact?.previouslyBinarySha256 === expected.previouslyBinarySha256;
  if (!identityMatches) {
    throw new Error("resume artifact does not match the current commit, product binary, or fixture bytes");
  }
  if (stableStringify(artifact.codexApp) !== stableStringify(expected.codexApp)) {
    throw new Error("resume artifact does not match the supplied Codex App evidence");
  }
  validateArtifactEvidenceBindings(artifact.codexApp, artifact.releaseEligibility?.seriousStaleApplications, outputPath);
  if (
    stableStringify(artifact.releaseEligibility?.seriousStaleApplications) !==
    stableStringify(expected.seriousStaleApplications)
  ) {
    throw new Error("resume artifact does not match the supplied stale-application evaluation evidence");
  }
  if (
    artifact.localMappedRegression?.artifactSha256 !== expected.mappedArtifactSha256 ||
    artifact.localMappedRegression?.gitCommit !== expected.gitCommit
  ) {
    throw new Error("resume artifact does not match the mapped regression evidence");
  }
  const mappedEvidencePath = join(evidenceRoot, "mapped-compatibility-results.json");
  if (
    !existsSync(mappedEvidencePath) ||
    !lstatSync(mappedEvidencePath).isFile() ||
    lstatSync(mappedEvidencePath).isSymbolicLink() ||
    sha256(readFileSync(mappedEvidencePath)) !== expected.mappedArtifactSha256
  ) {
    throw new Error("resume mapped regression evidence is missing or has changed");
  }

  const expectedIds = new Set(expected.scenarioIds);
  const scenarioContracts = new Map(expected.scenarios.map((scenario) => [scenario.id, scenario]));
  const expectedRuns = new Map(expected.codexRuns.map((run) => [run.role, run]));
  const actualRuns = artifact.codexCli?.runs;
  if (!Array.isArray(actualRuns) || actualRuns.length !== expectedRuns.size) {
    throw new Error("resume artifact has an unexpected Codex run set");
  }
  let completedRuns = 0;
  let dataLossEvents = 0;
  const seenRoles = new Set();
  for (const run of actualRuns) {
    const expectedRun = expectedRuns.get(run.role);
    if (
      !expectedRun ||
      seenRoles.has(run.role) ||
      run.version !== expectedRun.version ||
      run.binarySha256 !== expectedRun.binarySha256
    ) {
      throw new Error(`resume artifact Codex binding changed for role ${run.role ?? "unknown"}`);
    }
    seenRoles.add(run.role);
    const seen = new Set();
    const passed = [];
    for (const scenario of Array.isArray(run.scenarios) ? run.scenarios : []) {
      if (!expectedIds.has(scenario.id) || seen.has(scenario.id)) {
        throw new Error(`resume artifact contains an unknown or duplicate scenario ${scenario.id ?? "unknown"}`);
      }
      seen.add(scenario.id);
      if (scenario.status !== "passed") continue;
      const contract = scenarioContracts.get(scenario.id);
      if (
        scenario.dataLossEvents !== 0 ||
        stableStringify(scenario.codexExecution) !== stableStringify(expected.codexExecution) ||
        !REQUIRED_RECONSTRUCTION.every((field) => scenario.reconstruction?.[field] === true) ||
        scenario.scenarioAssertion?.status !== "passed" ||
        scenario.category !== contract?.category ||
        scenario.scenarioAssertion?.testTarget !== contract?.testTarget ||
        scenario.scenarioAssertion?.testFilter !== contract?.testFilter ||
        scenario.scenarioAssertion?.expectation !== contract?.expectation ||
        scenario.scenarioAssertion?.mappedArtifactSha256 !== expected.mappedArtifactSha256
      ) {
        throw new Error(`resume scenario ${run.role}/${scenario.id} is not a valid passed checkpoint`);
      }
      const expectedEvidencePath = join(evidenceRoot, run.role, scenario.id, "evidence.json");
      const recordedEvidencePath = resolve(dirname(outputPath), scenario.evidencePath ?? "");
      if (
        recordedEvidencePath !== expectedEvidencePath ||
        !existsSync(expectedEvidencePath) ||
        !lstatSync(expectedEvidencePath).isFile() ||
        lstatSync(expectedEvidencePath).isSymbolicLink() ||
        sha256(readFileSync(expectedEvidencePath)) !== scenario.evidenceSha256
      ) {
        throw new Error(`resume evidence for ${run.role}/${scenario.id} is missing or has changed`);
      }
      const evidence = readJson(expectedEvidencePath);
      if (
        evidence.role !== run.role ||
        evidence.codexVersion !== run.version ||
        stableStringify(evidence.codexExecution) !== stableStringify(expected.codexExecution) ||
        evidence.scenario?.id !== scenario.id ||
        evidence.scenario?.category !== scenario.category ||
        stableStringify(evidence.scenarioAssertion) !== stableStringify(scenario.scenarioAssertion) ||
        evidence.verification?.passed !== true ||
        !REQUIRED_RECONSTRUCTION.every((field) => evidence.verification?.reconstruction?.[field] === true)
      ) {
        throw new Error(`resume evidence contract for ${run.role}/${scenario.id} is invalid`);
      }
      passed.push(scenario);
      completedRuns += 1;
      dataLossEvents += scenario.dataLossEvents;
    }
    run.scenarios = passed;
  }
  artifact.generatedAt = new Date().toISOString();
  artifact.liveCodexWorkflowMatrix = { status: "running", requiredRuns: 60, completedRuns };
  artifact.transparentCaptureReleaseGate = {
    eligible: false,
    reason: `${60 - completedRuns} authenticated workflows remain after checkpoint validation`,
  };
  artifact.releaseEligibility = {
    ...artifact.releaseEligibility,
    eligible: false,
    dataLossEvents,
  };
  return artifact;
}

export function finalizeCompletedArtifact(
  artifact,
  expected,
  outputPath,
  evidenceRoot,
  seriousStaleApplications,
) {
  const revalidated = revalidateCompletedArtifactForFinalize(artifact, expected, outputPath, evidenceRoot);
  return attachStaleEvaluation(revalidated, expected.scenarioIds, outputPath, seriousStaleApplications);
}

export function revalidateCompletedArtifactForFinalize(artifact, expected, outputPath, evidenceRoot) {
  const expectedScenarioIds = expected.scenarioIds;
  const existingStale = artifact.releaseEligibility?.seriousStaleApplications;
  if (existingStale?.status !== "unmeasured") {
    throw new Error("stale-evaluation finalize requires an unmeasured artifact and cannot replace measured evidence");
  }
  if (
    artifact.liveCodexWorkflowMatrix?.status !== "complete" ||
    artifact.liveCodexWorkflowMatrix?.requiredRuns !== 60 ||
    artifact.liveCodexWorkflowMatrix?.completedRuns !== 60 ||
    artifact.releaseEligibility?.eligible !== false ||
    artifact.transparentCaptureReleaseGate?.eligible !== false ||
    artifact.releaseEligibility?.dataLossEvents !== 0 ||
    !hasCompleteWorkflowMatrix(artifact, expectedScenarioIds)
  ) {
    throw new Error("stale-evaluation finalize requires a complete, ineligible, lossless 60-run artifact");
  }
  const revalidated = prepareResumeArtifact(
    artifact,
    { ...expected, seriousStaleApplications: existingStale },
    outputPath,
    evidenceRoot,
  );
  if (
    revalidated.liveCodexWorkflowMatrix.completedRuns !== 60 ||
    !hasCompleteWorkflowMatrix(revalidated, expectedScenarioIds) ||
    revalidated.releaseEligibility.dataLossEvents !== 0
  ) {
    throw new Error("stale-evaluation finalize lost one or more validated workflow checkpoints");
  }
  return revalidated;
}

function attachStaleEvaluation(revalidated, expectedScenarioIds, outputPath, seriousStaleApplications) {
  if (
    seriousStaleApplications?.status !== "measured" ||
    seriousStaleApplications.scenariosEvaluated !== 60 ||
    !Number.isInteger(seriousStaleApplications.count) ||
    seriousStaleApplications.count < 0
  ) {
    throw new Error("stale-evaluation finalize requires a measured evaluator covering all 60 workflows");
  }
  validateArtifactEvidenceBindings(revalidated.codexApp, seriousStaleApplications, outputPath);
  revalidated.releaseEligibility.seriousStaleApplications = seriousStaleApplications;
  const eligible = computeEligibility(revalidated, expectedScenarioIds);
  revalidated.generatedAt = new Date().toISOString();
  revalidated.liveCodexWorkflowMatrix = { status: "complete", requiredRuns: 60, completedRuns: 60 };
  revalidated.transparentCaptureReleaseGate = eligible
    ? { eligible: true, reason: "all 60 workflows and separately retained release evidence passed" }
    : { eligible: false, reason: releaseIneligibilityReason(revalidated) };
  revalidated.releaseEligibility.eligible = eligible;
  return revalidated;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const matrix = readJson(MATRIX_PATH);
  const contract = readJson(CONTRACT_PATH);
  const { scenarios, categories } = validateFixtureContract(matrix, contract);
  if (args.dryRun) {
    const plan = buildDryRunPlan(matrix, contract);
    writeJson(args.output ?? join(ROOT, "outputs/live-compatibility-dry-run.json"), plan);
    process.stdout.write(`dry-run live compatibility plan validated: ${args.output ?? join(ROOT, "outputs/live-compatibility-dry-run.json")}\n`);
    return;
  }

  if (args.finalizeStaleEvaluation) requireFinalizeArguments(args, contract);
  else requireLiveArguments(args, contract);
  const latestVersion = await validateCodexBinary(args.latestBin, "latest");
  const previousVersion = await validateCodexBinary(args.previousBin, "previous");
  if (compareSemver(latestVersion, previousVersion) <= 0) {
    throw new Error(`latest Codex ${latestVersion} must be newer than previous ${previousVersion}`);
  }
  const productVersion = await validatePreviouslyBinary(args.previouslyBin);
  const gitCommit = checkedText("git", ["-C", ROOT, "rev-parse", "HEAD"]).trim();
  if (!/^[0-9a-f]{40}$/i.test(gitCommit)) throw new Error("live gate requires a committed Git SHA");
  if (checkedText("git", ["-C", ROOT, "status", "--porcelain"]).trim() !== "") {
    throw new Error("live gate requires a clean committed checkout");
  }
  if (process.platform !== "darwin" || process.arch !== "arm64") {
    throw new Error("release live compatibility must run on Apple Silicon macOS");
  }
  const mappedRegression = validateMappedArtifact(
    args.mappedArtifact,
    scenarios,
    gitCommit,
    productVersion,
    latestVersion,
    previousVersion,
  );

  const sourceAuth = args.finalizeStaleEvaluation ? null : join(args.codexHome, contract.auth.sourceFile);
  if (!args.finalizeStaleEvaluation && !existsSync(sourceAuth)) {
    throw new Error(`authenticated CODEX_HOME omitted ${contract.auth.sourceFile}`);
  }
  if (existsSync(args.output) && !args.resume && !args.finalizeStaleEvaluation) {
    throw new Error("--output already exists; use a new path so partial and reviewed evidence cannot be mixed");
  }
  const evidenceRoot = resolve(args.evidenceDir ?? `${args.output}.evidence`);
  requireContainedEvidenceRoot(args.output, evidenceRoot);
  if (existsSync(evidenceRoot) && !args.resume && !args.finalizeStaleEvaluation) {
    throw new Error("evidence directory already exists; use a new path so stale scenario files cannot be retained");
  }
  if (args.resume && (!existsSync(args.output) || !existsSync(evidenceRoot))) {
    throw new Error("--resume requires an existing output artifact and evidence directory");
  }
  if (args.finalizeStaleEvaluation && (!existsSync(args.output) || !existsSync(evidenceRoot))) {
    throw new Error("--finalize-stale-evaluation requires an existing output artifact and evidence directory");
  }
  if (args.finalizeStaleEvaluation) {
    const codexRuns = [
      { role: "latest", version: latestVersion, binarySha256: sha256(readFileSync(args.latestBin)), scenarios: [] },
      { role: "previous", version: previousVersion, binarySha256: sha256(readFileSync(args.previousBin)), scenarios: [] },
    ];
    const existingArtifact = readJson(args.output);
    const codexApp = buildAppEvidence(args, args.output, evidenceRoot, true, existingArtifact?.codexApp?.previous);
    const expected = {
      productVersion,
      gitCommit,
      runner: { os: "macOS", arch: "arm64" },
      codexExecution: contract.execution,
      categories,
      fixtureContractSha256: sha256(readFileSync(CONTRACT_PATH)),
      scenarioMatrixSha256: sha256(readFileSync(MATRIX_PATH)),
      previouslyBinarySha256: sha256(readFileSync(args.previouslyBin)),
      codexRuns,
      codexApp,
      mappedArtifactSha256: mappedRegression.artifactSha256,
      scenarioIds: scenarios.map((scenario) => scenario.id),
      scenarios,
    };
    const revalidated = revalidateCompletedArtifactForFinalize(
      existingArtifact,
      expected,
      args.output,
      evidenceRoot,
    );
    const seriousStaleApplications = buildStaleEvaluationEvidence({
      args,
      outputPath: args.output,
      evidenceRoot,
      resume: false,
      allowExistingIdentical: true,
      productVersion,
      gitCommit,
    });
    const artifact = attachStaleEvaluation(
      revalidated,
      expected.scenarioIds,
      args.output,
      seriousStaleApplications,
    );
    writeJson(args.output, artifact);
    if (artifact.releaseEligibility.eligible) {
      checked(process.execPath, [
        join(ROOT, "scripts/validate-live-compatibility.mjs"),
        args.output,
        "--commit",
        gitCommit,
        "--product-version",
        productVersion,
      ]);
    }
    const bundlePath = packageEvidenceBundle(
      args.output,
      evidenceRoot,
      args.bundle ?? `${args.output}.finalized.tar.gz`,
    );
    process.stdout.write(
      `${artifact.releaseEligibility.eligible ? "eligible" : "ineligible"} finalized live compatibility evidence written: ${args.output}\n` +
        `release evidence bundle: ${bundlePath}\n` +
        `bundle sha256: ${sha256(readFileSync(bundlePath))}\n`,
    );
    if (!artifact.releaseEligibility.eligible) {
      throw new Error(`finalized live evidence remains release-ineligible: ${releaseIneligibilityReason(artifact)}`);
    }
    return;
  }
  const tempRoot = mkdtempSync(join(tmpdir(), "previously-live-compat-"));
  chmodSync(tempRoot, 0o700);
  try {
  mkdirSync(evidenceRoot, { recursive: true, mode: 0o700 });
  const mappedEvidencePath = join(evidenceRoot, "mapped-compatibility-results.json");
  if (!args.resume) {
    copyFileSync(args.mappedArtifact, mappedEvidencePath);
    chmodSync(mappedEvidencePath, 0o600);
  }
  mappedRegression.evidencePath = relative(dirname(args.output), mappedEvidencePath);
  const authSeeds = {};
  for (const role of ["latest", "previous"]) {
    const seed = join(tempRoot, `${role}-auth-seed`);
    mkdirSync(seed, { mode: 0o700 });
    copyFileSync(sourceAuth, join(seed, "auth.json"));
    chmodSync(join(seed, "auth.json"), 0o600);
    authSeeds[role] = seed;
  }

  const codexRuns = [
    { role: "latest", version: latestVersion, binarySha256: sha256(readFileSync(args.latestBin)), scenarios: [] },
    { role: "previous", version: previousVersion, binarySha256: sha256(readFileSync(args.previousBin)), scenarios: [] },
  ];
  const existingArtifact = args.resume ? readJson(args.output) : null;
  const codexApp = buildAppEvidence(args, args.output, evidenceRoot, args.resume, existingArtifact?.codexApp?.previous);
  const seriousStaleApplications = buildStaleEvaluationEvidence({
    args,
    outputPath: args.output,
    evidenceRoot,
    resume: args.resume,
    productVersion,
    gitCommit,
  });
  let artifact = {
    schemaVersion: 1,
    evidenceClass: "live_codex_workflow_matrix",
    product: "PreviouslyOn",
    productVersion,
    gitCommit,
    generatedAt: new Date().toISOString(),
    supportMode: "explicit_run_and_import",
    runner: { os: "macOS", arch: "arm64" },
    codexExecution: contract.execution,
    fixtureContractSha256: sha256(readFileSync(CONTRACT_PATH)),
    scenarioMatrixSha256: sha256(readFileSync(MATRIX_PATH)),
    previouslyBinarySha256: sha256(readFileSync(args.previouslyBin)),
    codexCli: {
      runs: codexRuns,
    },
    codexApp,
    localMappedRegression: mappedRegression,
    liveCodexWorkflowMatrix: { status: "running", requiredRuns: 60, completedRuns: 0 },
    transparentCaptureReleaseGate: { eligible: false, reason: "60 live workflows are incomplete" },
    releaseEligibility: { eligible: false, dataLossEvents: 0, seriousStaleApplications },
    categories,
  };
  if (args.resume) {
    artifact = prepareResumeArtifact(
      existingArtifact,
      {
        productVersion,
        gitCommit,
        runner: { os: "macOS", arch: "arm64" },
        codexExecution: contract.execution,
        categories,
        fixtureContractSha256: sha256(readFileSync(CONTRACT_PATH)),
        scenarioMatrixSha256: sha256(readFileSync(MATRIX_PATH)),
        previouslyBinarySha256: sha256(readFileSync(args.previouslyBin)),
        codexRuns,
        codexApp,
        seriousStaleApplications,
        mappedArtifactSha256: mappedRegression.artifactSha256,
        scenarioIds: scenarios.map((scenario) => scenario.id),
        scenarios,
      },
      args.output,
      evidenceRoot,
    );
  }
  writeJson(args.output, artifact);

  let fatalError = null;
  for (const run of artifact.codexCli.runs) {
      const binary = run.role === "latest" ? args.latestBin : args.previousBin;
      await verifyAuthenticated(binary, authSeeds[run.role], tempRoot);
      for (const scenario of scenarios) {
        if (run.scenarios.some((result) => result.id === scenario.id && result.status === "passed")) continue;
        const fixture = buildWorkflowFixture(scenario, contract);
        try {
          const result = await executeWorkflow({
            args,
            binary,
            role: run.role,
            version: run.version,
            fixture,
            tempRoot,
            authSeed: authSeeds[run.role],
            evidenceRoot,
            mappedArtifactSha256: mappedRegression.artifactSha256,
            codexExecution: contract.execution,
          });
          run.scenarios.push(result);
          artifact.releaseEligibility.dataLossEvents += result.dataLossEvents;
          artifact.liveCodexWorkflowMatrix.completedRuns += 1;
          writeJson(args.output, artifact);
        } catch (error) {
          run.scenarios.push({
            id: fixture.id,
            category: fixture.category,
            status: "failed",
            error: safeError(error),
            reconstruction: Object.fromEntries(REQUIRED_RECONSTRUCTION.map((field) => [field, false])),
          });
          fatalError = error;
          writeJson(args.output, artifact);
          break;
        }
      }
      if (fatalError) break;
  }

  const expectedIds = scenarios.map((scenario) => scenario.id);
  const workflowsComplete = !fatalError && hasCompleteWorkflowMatrix(artifact, expectedIds);
  const eligible = workflowsComplete && computeEligibility(artifact, expectedIds);
  artifact.generatedAt = new Date().toISOString();
  artifact.liveCodexWorkflowMatrix.status = workflowsComplete ? "complete" : "failed";
  artifact.transparentCaptureReleaseGate = eligible
    ? { eligible: true, reason: "all 60 workflows and separately retained release evidence passed" }
    : {
        eligible: false,
        reason: fatalError
          ? safeError(fatalError)
          : workflowsComplete
            ? releaseIneligibilityReason(artifact)
            : "one or more required live workflows failed",
      };
  artifact.releaseEligibility.eligible = eligible;
  writeJson(args.output, artifact);
  if (eligible) {
    checked(process.execPath, [
      join(ROOT, "scripts/validate-live-compatibility.mjs"),
      args.output,
      "--commit",
      gitCommit,
      "--product-version",
      productVersion,
    ]);
  }
  const bundlePath = packageEvidenceBundle(
    args.output,
    evidenceRoot,
    args.bundle ?? `${args.output}.tar.gz`,
  );
  process.stdout.write(
    `${eligible ? "eligible" : "ineligible"} 60-run live compatibility evidence written: ${args.output}\n` +
      `release evidence bundle: ${bundlePath}\n` +
      `bundle sha256: ${sha256(readFileSync(bundlePath))}\n`,
  );
  if (!eligible) throw new Error(`live workflows were retained but automatic release eligibility failed closed: ${releaseIneligibilityReason(artifact)}`);
  } finally {
    if (!args.keepTemporary) rmSync(tempRoot, { recursive: true, force: true });
  }
}

async function executeWorkflow({
  args,
  binary,
  role,
  version,
  fixture,
  tempRoot,
  authSeed,
  evidenceRoot,
  mappedArtifactSha256,
  codexExecution,
}) {
  const work = join(tempRoot, `${role}-${fixture.runtimeSlug}`);
  const repo = join(work, "repo");
  const dataDir = join(work, "data");
  const codexHome = join(work, "codex-home");
  const shimDir = join(work, "bin");
  mkdirSync(repo, { recursive: true, mode: 0o700 });
  mkdirSync(codexHome, { mode: 0o700 });
  mkdirSync(shimDir, { mode: 0o700 });
  copyFileSync(join(authSeed, "auth.json"), join(codexHome, "auth.json"));
  chmodSync(join(codexHome, "auth.json"), 0o600);
  symlinkSync(binary, join(shimDir, "codex"));
  writeFileSync(join(repo, "state.txt"), "baseline\n", { mode: 0o600 });
  writeFileSync(
    join(repo, "verify.sh"),
    '#!/bin/sh\nset -eu\nexpected="$1"\nactual="$(cat state.txt)"\n[ "$actual" = "$expected" ]\nprintf "VERIFY_OK %s\\n" "$expected"\n',
    { mode: 0o700 },
  );
  checked("git", ["init", "--initial-branch=main"], { cwd: repo });
  checked("git", ["add", "state.txt", "verify.sh"], { cwd: repo });
  checked("git", ["-c", "user.name=PreviouslyOn Live Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-m", "fixture baseline"], { cwd: repo });
  const baselineHead = checkedText("git", ["rev-parse", "HEAD"], { cwd: repo }).trim();
  const env = isolatedEnvironment({ tempRoot: work, codexHome, shimDir });
  const evidenceDir = join(evidenceRoot, role, fixture.id);
  mkdirSync(evidenceDir, { recursive: true, mode: 0o700 });
  let installed = false;
  try {
    checked(args.previouslyBin, ["--data-dir", dataDir, "setup", "codex", "--repo", repo], { env });
    installed = true;
    const initialFinalPath = join(work, "initial-final.txt");
    const initial = await runCaptured(
      args.previouslyBin,
      [
        "--data-dir", dataDir, "run", "codex", "--repo", repo, "--",
        "exec", "-C", repo, "--json", "--color", "never", "--sandbox", codexExecution.sandbox,
        "--dangerously-bypass-hook-trust", "--output-last-message", initialFinalPath,
        "--model", codexExecution.model,
        "-c", `model_reasoning_effort="${codexExecution.reasoningEffort}"`,
        "-c", `sandbox_mode="${codexExecution.sandbox}"`,
        "--strict-config", fixture.initialPrompt,
      ],
      { env, cwd: repo, timeoutMs: args.timeoutMs },
    );
    if (initial.code !== 0) throw new Error(`initial Codex turn exited ${initial.code}: ${tail(initial.stderr)}`);
    const initialEvents = parseJsonLines(initial.stdout);
    const sessionId = extractSessionId(initialEvents);
    const initialContent = readFileSync(join(repo, "state.txt"), "utf8");
    const initialFinalText = readFileSync(initialFinalPath, "utf8");
    const initialDiff = checkedText("git", ["diff", "--no-ext-diff", "--binary"], { cwd: repo });

    const resumeFinalPath = join(work, "resume-final.txt");
    const resume = await runCaptured(
      args.previouslyBin,
      [
        "--data-dir", dataDir, "run", "codex", "--repo", repo, "--",
        "exec", "resume", "--json", "--dangerously-bypass-hook-trust",
        "--output-last-message", resumeFinalPath, "--model", codexExecution.model,
        "-c", `model_reasoning_effort="${codexExecution.reasoningEffort}"`,
        "-c", `sandbox_mode="${codexExecution.sandbox}"`, "--strict-config",
        sessionId, fixture.resumePrompt,
      ],
      { env, cwd: repo, timeoutMs: args.timeoutMs },
    );
    if (resume.code !== 0) throw new Error(`resume Codex turn exited ${resume.code}: ${tail(resume.stderr)}`);
    const resumeEvents = parseJsonLines(resume.stdout);
    const resumedSessionId = extractSessionId(resumeEvents, sessionId);
    if (resumedSessionId !== sessionId) throw new Error("exec resume returned a different stable session ID");
    const resumeContent = readFileSync(join(repo, "state.txt"), "utf8");
    const resumeFinalText = readFileSync(resumeFinalPath, "utf8");
    const resumeDiff = checkedText("git", ["diff", "--no-ext-diff", "--binary"], { cwd: repo });
    const gitStatus = checkedText("git", ["status", "--short"], { cwd: repo });
    const headAfter = checkedText("git", ["rev-parse", "HEAD"], { cwd: repo }).trim();
    if (headAfter !== baselineHead) throw new Error("Codex unexpectedly committed during compatibility workflow");

    const explicitImport = checkedText(
      args.previouslyBin,
      ["--data-dir", dataDir, "import", "codex", "--repo", repo],
      { env, cwd: repo },
    );
    const importReport = JSON.parse(explicitImport);
    const exportText = checkedText(args.previouslyBin, ["--data-dir", dataDir, "export", "--format", "json"], { env, cwd: repo });
    const exportData = JSON.parse(exportText);
    const verification = verifyScenarioEvidence({
      fixture,
      exportData,
      sessionId,
      initialFinalText,
      resumeFinalText,
      initialContent,
      resumeContent,
      gitStatus,
      codexExecution,
    });
    if (!verification.passed) throw new Error(`reconstruction failed: ${JSON.stringify(verification.reconstruction)}`);
    const dataLossEvents = countDataLoss(initial.stderr) + countDataLoss(resume.stderr);
    if (dataLossEvents !== 0) throw new Error("PreviouslyOn reported a data-loss diagnostic");

    const initialJsonl = `${initialEvents.map((event) => JSON.stringify(event)).join("\n")}\n`;
    const resumeJsonl = `${resumeEvents.map((event) => JSON.stringify(event)).join("\n")}\n`;
    const evidence = {
      schemaVersion: 1,
      role,
      codexVersion: version,
      codexExecution,
      scenario: { id: fixture.id, category: fixture.category },
      scenarioAssertion: {
        status: "passed",
        testTarget: fixture.testTarget,
        testFilter: fixture.testFilter,
        expectation: fixture.expectation,
        mappedArtifactSha256,
      },
      groundTruth: {
        baselineHead,
        headAfter,
        initialContentSha256: sha256(initialContent),
        resumeContentSha256: sha256(resumeContent),
        initialDiffSha256: sha256(initialDiff),
        resumeDiffSha256: sha256(resumeDiff),
        gitStatus: gitStatus.trim(),
        initialFinalSha256: sha256(initialFinalText),
        resumeFinalSha256: sha256(resumeFinalText),
      },
      jsonlObservation: {
        initialEventCount: initialEvents.length,
        resumeEventCount: resumeEvents.length,
        initialSha256: sha256(initialJsonl),
        resumeSha256: sha256(resumeJsonl),
      },
      previouslyObservation: {
        exportSha256: sha256(exportText),
        importedThreads: importReport.importedThreads,
        coverageSha256: sha256(stableStringify(importReport.coverage ?? null)),
        noticeCount: Array.isArray(importReport.notices) ? importReport.notices.length : 0,
        noticesSha256: sha256(stableStringify(importReport.notices ?? [])),
      },
      verification,
    };
    const evidenceText = `${stableStringify(evidence)}\n`;
    writePrivate(join(evidenceDir, "evidence.json"), evidenceText);
    return {
      id: fixture.id,
      category: fixture.category,
      status: "passed",
      codexExecution,
      reconstruction: verification.reconstruction,
      scenarioAssertion: evidence.scenarioAssertion,
      dataLossEvents,
      evidenceSha256: sha256(evidenceText),
      evidencePath: relative(dirname(args.output), join(evidenceDir, "evidence.json")),
    };
  } finally {
    if (existsSync(join(codexHome, "auth.json"))) {
      copyFileSync(join(codexHome, "auth.json"), join(authSeed, "auth.json"));
      chmodSync(join(authSeed, "auth.json"), 0o600);
    }
    if (installed) {
      await runCaptured(args.previouslyBin, ["--data-dir", dataDir, "uninstall", "codex"], {
        env,
        cwd: repo,
        timeoutMs: 30_000,
      }).catch(() => {});
    }
  }
}

async function verifyAuthenticated(binary, authSeed, tempRoot) {
  const env = isolatedEnvironment({ tempRoot, codexHome: authSeed, shimDir: dirname(binary) });
  const result = await runCaptured(binary, ["login", "status"], { env, timeoutMs: 30_000 });
  if (result.code !== 0) throw new Error("supplied CODEX_HOME is not authenticated for a supplied Codex binary");
}

async function validateCodexBinary(binary, role) {
  requireExecutable(binary, `--${role}-bin`);
  const versionResult = await runCaptured(binary, ["--version"], { timeoutMs: 30_000 });
  if (versionResult.code !== 0) throw new Error(`${role} Codex --version exited ${versionResult.code}`);
  const version = versionResult.stdout.trim().split(/\s+/).at(-1);
  if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) throw new Error(`${role} Codex binary did not report stable semver`);
  const help = await runCaptured(binary, ["exec", "resume", "--help"], { timeoutMs: 30_000 });
  if (help.code !== 0 || !help.stdout.includes("Resume a previous session")) {
    throw new Error(`${role} Codex binary does not expose exec resume`);
  }
  return version;
}

async function validatePreviouslyBinary(binary) {
  requireExecutable(binary, "--previously-bin");
  const result = await runCaptured(binary, ["--version"], { timeoutMs: 30_000 });
  if (result.code !== 0) throw new Error("PreviouslyOn binary --version failed");
  const version = result.stdout.trim().split(/\s+/).at(-1);
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version ?? "")) {
    throw new Error("PreviouslyOn binary returned an invalid version");
  }
  return version;
}

function findEvent(events, kind, needle, sessionId) {
  return events.find(
    (event) => event.kind === kind && event.session_id === sessionId && JSON.stringify(event.payload).includes(needle),
  );
}

function findAssistantFinal(events, needle, sessionId) {
  return (
    findEvent(events, "assistant_final", needle, sessionId) ??
    findEvent(events, "session_stopped", needle, sessionId)
  );
}

function findToolPair(events, sessionId, pathNeedle, valueNeedle) {
  const candidates = events.filter(
    (event) => event.kind === "tool_started" && event.session_id === sessionId && includesBoth(event.payload, pathNeedle, valueNeedle),
  );
  for (const pre of candidates) {
    const toolUseId = toolId(pre.payload);
    if (!toolUseId) continue;
    const post = events.find(
      (event) => event.kind === "tool_finished" && event.session_id === sessionId && toolId(event.payload) === toolUseId,
    );
    if (post) return { pre, post, toolUseId };
  }
  return null;
}

function toolId(payload) {
  return payload?.tool_use_id ?? payload?.toolUseId ?? payload?.call_id ?? payload?.callId ?? payload?.tool?.id ?? null;
}

function includesBoth(value, left, right) {
  const text = JSON.stringify(value);
  return text.includes(left) && text.includes(right);
}

function stableSource(event) {
  const sourceId = event.source_id ?? "";
  const missing = Array.isArray(event.coverage?.missing) ? event.coverage.missing : [];
  const reportsUnstableSource = missing.some((item) => /stable.*source.?id/i.test(item));
  const hookSource = /^src-[0-9a-f]{64}$/.test(sourceId);
  const appServerSource =
    /^codex-app-server:thread:[^:]+:(?:item:[^:]+:[a-z-]+|stop)$/.test(sourceId) &&
    !sourceId.includes("app-import-");
  return !reportsUnstableSource && (hookSource || appServerSource);
}

function extractSessionId(events, expected = null) {
  const candidates = new Set();
  for (const event of events) collectSessionIds(event, candidates);
  if (expected && candidates.has(expected)) return expected;
  const uuids = [...candidates].filter((value) => /^[0-9a-f]{8}-[0-9a-f-]{27,}$/i.test(value));
  if (uuids.length !== 1) throw new Error(`Codex JSONL did not expose exactly one stable session ID: ${[...candidates].join(",")}`);
  return uuids[0];
}

function collectSessionIds(value, output) {
  if (!value || typeof value !== "object") return;
  for (const [key, item] of Object.entries(value)) {
    if (["thread_id", "threadId", "session_id", "sessionId"].includes(key) && typeof item === "string") output.add(item);
    else collectSessionIds(item, output);
  }
}

function parseJsonLines(stdout) {
  const events = [];
  for (const line of stdout.split(/\r?\n/)) {
    if (!line.trim()) continue;
    try {
      const value = JSON.parse(line);
      if (value && typeof value === "object" && !Array.isArray(value)) events.push(value);
    } catch {
      // PreviouslyOn prints a multiline import report after the Codex JSONL stream. It is
      // intentionally excluded here and captured separately by the explicit import command.
    }
  }
  if (events.length === 0) throw new Error("Codex exec produced no parseable JSONL events");
  return events;
}

function isolatedEnvironment({ tempRoot, codexHome, shimDir }) {
  const env = {
    HOME: tempRoot,
    CODEX_HOME: codexHome,
    PATH: `${shimDir}:${process.env.PATH ?? "/usr/bin:/bin:/usr/sbin:/sbin"}`,
    TMPDIR: tempRoot,
    LANG: process.env.LANG ?? "C.UTF-8",
    LC_ALL: process.env.LC_ALL ?? "C.UTF-8",
  };
  for (const key of ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY", "SSL_CERT_FILE", "SSL_CERT_DIR"]) {
    if (process.env[key]) env[key] = process.env[key];
  }
  return env;
}

export function requireLiveArguments(args, contract) {
  if (args.confirm !== contract.confirmationPhrase) {
    throw new Error(`live execution requires --confirm ${contract.confirmationPhrase}`);
  }
  if (args.staleEvaluationArtifact || args.staleEvaluationSha256) {
    throw new Error(
      "--stale-evaluation-artifact and --stale-evaluation-sha256 are finalize-only; live run/resume must remain unmeasured",
    );
  }
  for (const key of [
    "latestBin",
    "previousBin",
    "previouslyBin",
    "mappedArtifact",
    "codexHome",
    "model",
    "reasoningEffort",
    "output",
    "codexAppCurrentBuild",
    "codexAppCurrentEvidence",
    "codexAppCurrentEvidenceSha256",
  ]) {
    if (!args[key]) throw new Error(`live execution omitted --${camelToKebab(key)}`);
  }
  requireBoundCodexExecution(args, contract);
  if (!isAbsolute(args.codexHome) || !isAbsolute(args.output)) {
    throw new Error("--codex-home and --output must be absolute paths");
  }
  if (basename(args.output) !== "live-compatibility.json") {
    throw new Error("--output basename must be live-compatibility.json");
  }
  if (args.bundle && !isAbsolute(args.bundle)) {
    throw new Error("--bundle must be an absolute path when provided");
  }
  if (args.evidenceDir && !isAbsolute(args.evidenceDir)) {
    throw new Error("--evidence-dir must be an absolute path when provided");
  }
  if (!isAbsolute(args.mappedArtifact) || !existsSync(args.mappedArtifact)) {
    throw new Error("--mapped-artifact must be an existing absolute path");
  }
  if (!/^[0-9a-f]{64}$/i.test(args.codexAppCurrentEvidenceSha256)) {
    throw new Error("--codex-app-current-evidence-sha256 must be 64 hexadecimal characters");
  }
  requireRegularAbsoluteFile(args.codexAppCurrentEvidence, "--codex-app-current-evidence");
  const hasPreviousBuild = Boolean(
    args.codexAppPreviousBuild || args.codexAppPreviousEvidence || args.codexAppPreviousEvidenceSha256,
  );
  const hasUnavailable = Boolean(args.codexAppPreviousUnavailableReason);
  if (hasPreviousBuild === hasUnavailable) {
    throw new Error("provide either previous App build/evidence or --codex-app-previous-unavailable-reason");
  }
  if (
    hasPreviousBuild &&
    (!args.codexAppPreviousBuild ||
      !args.codexAppPreviousEvidence ||
      !/^[0-9a-f]{64}$/i.test(args.codexAppPreviousEvidenceSha256 ?? ""))
  ) {
    throw new Error("previous App evidence requires build, retained evidence file, and SHA-256");
  }
  if (hasPreviousBuild) requireRegularAbsoluteFile(args.codexAppPreviousEvidence, "--codex-app-previous-evidence");
  if (
    args.codexAppPreviousUnavailableReason &&
    args.codexAppPreviousCheckedAt &&
    !Number.isFinite(Date.parse(args.codexAppPreviousCheckedAt))
  ) {
    throw new Error("--codex-app-previous-checked-at must be an ISO-8601 timestamp");
  }
}

function requireFinalizeArguments(args, contract) {
  if (args.resume) {
    throw new Error("--finalize-stale-evaluation is a distinct one-way mode and cannot be combined with --resume");
  }
  for (const key of [
    "latestBin",
    "previousBin",
    "previouslyBin",
    "mappedArtifact",
    "model",
    "reasoningEffort",
    "output",
    "codexAppCurrentBuild",
    "codexAppCurrentEvidence",
    "codexAppCurrentEvidenceSha256",
    "staleEvaluationArtifact",
    "staleEvaluationSha256",
  ]) {
    if (!args[key]) throw new Error(`stale-evaluation finalize omitted --${camelToKebab(key)}`);
  }
  requireBoundCodexExecution(args, contract);
  if (!isAbsolute(args.output) || basename(args.output) !== "live-compatibility.json") {
    throw new Error("--output must be an absolute path ending in live-compatibility.json");
  }
  if (args.bundle && !isAbsolute(args.bundle)) {
    throw new Error("--bundle must be an absolute path when provided");
  }
  if (args.evidenceDir && !isAbsolute(args.evidenceDir)) {
    throw new Error("--evidence-dir must be an absolute path when provided");
  }
  if (!isAbsolute(args.mappedArtifact) || !existsSync(args.mappedArtifact)) {
    throw new Error("--mapped-artifact must be an existing absolute path");
  }
  if (!/^[0-9a-f]{64}$/i.test(args.codexAppCurrentEvidenceSha256)) {
    throw new Error("--codex-app-current-evidence-sha256 must be 64 hexadecimal characters");
  }
  requireRegularAbsoluteFile(args.codexAppCurrentEvidence, "--codex-app-current-evidence");
  const hasPreviousBuild = Boolean(
    args.codexAppPreviousBuild || args.codexAppPreviousEvidence || args.codexAppPreviousEvidenceSha256,
  );
  const hasUnavailable = Boolean(args.codexAppPreviousUnavailableReason);
  if (hasPreviousBuild === hasUnavailable) {
    throw new Error("provide either previous App build/evidence or --codex-app-previous-unavailable-reason");
  }
  if (
    hasPreviousBuild &&
    (!args.codexAppPreviousBuild ||
      !args.codexAppPreviousEvidence ||
      !/^[0-9a-f]{64}$/i.test(args.codexAppPreviousEvidenceSha256 ?? ""))
  ) {
    throw new Error("previous App evidence requires build, retained evidence file, and SHA-256");
  }
  if (hasPreviousBuild) requireRegularAbsoluteFile(args.codexAppPreviousEvidence, "--codex-app-previous-evidence");
  if (
    args.codexAppPreviousUnavailableReason &&
    args.codexAppPreviousCheckedAt &&
    !Number.isFinite(Date.parse(args.codexAppPreviousCheckedAt))
  ) {
    throw new Error("--codex-app-previous-checked-at must be an ISO-8601 timestamp");
  }
  requireRegularAbsoluteFile(args.staleEvaluationArtifact, "--stale-evaluation-artifact");
  if (!/^[0-9a-f]{64}$/i.test(args.staleEvaluationSha256)) {
    throw new Error("--stale-evaluation-sha256 must be 64 hexadecimal characters");
  }
}

function requireContainedEvidenceRoot(outputPath, evidenceRoot) {
  const outputDirectory = dirname(resolve(outputPath));
  const relativeEvidenceRoot = relative(outputDirectory, evidenceRoot);
  if (
    relativeEvidenceRoot === "" ||
    relativeEvidenceRoot === ".." ||
    relativeEvidenceRoot.startsWith(`..${process.platform === "win32" ? "\\" : "/"}`) ||
    isAbsolute(relativeEvidenceRoot)
  ) {
    throw new Error("--evidence-dir must be a child of the output artifact directory");
  }
}

export function packageEvidenceBundle(artifactPath, evidenceRoot, bundlePath) {
  const artifact = resolve(artifactPath);
  const evidence = resolve(evidenceRoot);
  const bundle = resolve(bundlePath);
  if (!existsSync(artifact) || !existsSync(evidence)) {
    throw new Error("live artifact and evidence directory must exist before bundling");
  }
  if (existsSync(bundle)) {
    throw new Error("evidence bundle already exists; refusing to overwrite reviewed bytes");
  }
  requireContainedEvidenceRoot(artifact, evidence);
  const base = dirname(artifact);
  const evidenceRelative = relative(base, evidence);
  const result = spawnSync(
    "tar",
    ["-czf", bundle, "-C", base, basename(artifact), evidenceRelative],
    { encoding: "utf8", maxBuffer: MAX_CAPTURE_BYTES },
  );
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`tar failed to create evidence bundle: ${tail(result.stderr ?? "")}`);
  }
  return bundle;
}

export function validateMappedArtifact(path, scenarios, gitCommit, productVersion, latestVersion, previousVersion) {
  const bytes = readFileSync(path);
  const artifact = JSON.parse(bytes.toString("utf8"));
  if (
    artifact.schemaVersion !== 1 ||
    artifact.product !== "PreviouslyOn" ||
    artifact.productVersion !== productVersion ||
    artifact.evidenceClass !== "local_mapped_regression_plus_live_app_server_schema_probe" ||
    artifact.supportMode !== "explicit_run_and_import"
  ) {
    throw new Error("mapped artifact has an unsupported identity or evidence class");
  }
  if (artifact.gitCommit !== gitCommit) {
    throw new Error("mapped artifact was not produced from the live harness commit");
  }
  if (artifact.gitTreeState !== "clean") {
    throw new Error("mapped artifact was not produced from a clean source tree");
  }
  if (artifact.scenarioMatrixSha256 !== sha256(readFileSync(MATRIX_PATH))) {
    throw new Error("mapped artifact does not cover the current scenario matrix bytes");
  }
  const expectedIds = scenarios.map((scenario) => scenario.id);
  if (
    artifact.localMappedRegression?.scenarioCount !== expectedIds.length ||
    artifact.liveCodexWorkflowMatrix?.status !== "not_run" ||
    artifact.transparentCaptureReleaseGate?.eligible !== false
  ) {
    throw new Error("mapped artifact does not preserve its separate ineligible evidence boundary");
  }
  const runs = artifact.localMappedRegression?.runs;
  if (!Array.isArray(runs) || runs.length !== 2) {
    throw new Error("mapped artifact must contain two regression runs");
  }
  const byVersion = new Map(runs.map((run) => [run.codexVersionSlot, run]));
  const contracts = new Map(scenarios.map((scenario) => [scenario.id, scenario]));
  for (const version of [latestVersion, previousVersion]) {
    const run = byVersion.get(version);
    if (
      !run ||
      !Array.isArray(run.passed) ||
      run.failed?.length !== 0 ||
      !Array.isArray(run.scenarioResults) ||
      run.scenarioResults.length !== expectedIds.length
    ) {
      throw new Error(`mapped regression did not pass for Codex ${version}`);
    }
    const seenResults = new Set();
    for (const result of run.scenarioResults) {
      const contract = contracts.get(result.id);
      if (
        !contract ||
        seenResults.has(result.id) ||
        result.testTarget !== contract.testTarget ||
        result.testFilter !== contract.testFilter ||
        result.expectation !== contract.expectation ||
        result.status !== "passed" ||
        result.exitCode !== 0
      ) {
        throw new Error(`mapped regression recorded an invalid actual row outcome for Codex ${version}`);
      }
      seenResults.add(result.id);
    }
    const passed = new Set(run.passed);
    if (
      seenResults.size !== expectedIds.length ||
      passed.size !== expectedIds.length ||
      expectedIds.some((id) => !passed.has(id))
    ) {
      throw new Error(`mapped regression omitted scenario coverage for Codex ${version}`);
    }
  }
  return {
    status: "separate_artifact_passed",
    artifactSha256: sha256(bytes),
    gitCommit,
    scenarioCount: expectedIds.length,
    limitation: "Scenario-specific internal assertions are mapped regressions; authenticated Codex turns separately prove reconstruction and source linkage.",
  };
}

function buildAppEvidence(args, outputPath, evidenceRoot, resume, existingPrevious = null) {
  const currentDocument = retainJsonEvidence({
    sourcePath: args.codexAppCurrentEvidence,
    destinationPath: join(evidenceRoot, "codex-app", "current.json"),
    expectedSha256: args.codexAppCurrentEvidenceSha256,
    label: "current Codex App evidence",
    resume,
  });
  if (
    currentDocument.schemaVersion !== 1 ||
    currentDocument.evidenceClass !== "codex_app_verification" ||
    currentDocument.product !== "Codex App" ||
    currentDocument.build !== args.codexAppCurrentBuild ||
    !Number.isFinite(Date.parse(currentDocument.checkedAt))
  ) {
    throw new Error("current Codex App retained evidence identity is invalid or mismatched");
  }
  if (!["passed", "degraded"].includes(currentDocument.status)) {
    throw new Error("current Codex App retained evidence status must be passed or degraded");
  }
  const currentEvidencePath = relative(dirname(outputPath), join(evidenceRoot, "codex-app", "current.json"));
  const previous = args.codexAppPreviousUnavailableReason
    ? {
        status: "unavailable",
        reason: args.codexAppPreviousUnavailableReason,
        checkedAt: args.codexAppPreviousCheckedAt ?? existingPrevious?.checkedAt ?? new Date().toISOString(),
      }
    : buildPreviousAppEvidence(args, outputPath, evidenceRoot, resume);
  return {
    current: {
      status: currentDocument.status,
      build: args.codexAppCurrentBuild,
      evidenceSha256: args.codexAppCurrentEvidenceSha256.toLowerCase(),
      evidencePath: currentEvidencePath,
    },
    previous,
  };
}

function buildPreviousAppEvidence(args, outputPath, evidenceRoot, resume) {
  const destinationPath = join(evidenceRoot, "codex-app", "previous.json");
  const document = retainJsonEvidence({
    sourcePath: args.codexAppPreviousEvidence,
    destinationPath,
    expectedSha256: args.codexAppPreviousEvidenceSha256,
    label: "previous Codex App evidence",
    resume,
  });
  if (
    document.schemaVersion !== 1 ||
    document.evidenceClass !== "codex_app_verification" ||
    document.product !== "Codex App" ||
    document.build !== args.codexAppPreviousBuild ||
    document.status !== "passed" ||
    !Number.isFinite(Date.parse(document.checkedAt))
  ) {
    throw new Error("previous Codex App retained evidence must match the supplied build and have passed status");
  }
  return {
    status: "passed",
    build: args.codexAppPreviousBuild,
    evidenceSha256: args.codexAppPreviousEvidenceSha256.toLowerCase(),
    evidencePath: relative(dirname(outputPath), destinationPath),
  };
}

function buildStaleEvaluationEvidence({
  args,
  outputPath,
  evidenceRoot,
  resume,
  allowExistingIdentical = false,
  productVersion,
  gitCommit,
}) {
  if (!args.staleEvaluationArtifact) {
    return {
      status: "unmeasured",
      reason: "no separately reviewed serious-stale-application evaluator artifact was supplied",
    };
  }
  const destinationPath = join(evidenceRoot, "evaluations", "serious-stale-applications.json");
  const sourceBytes = readFileSync(args.staleEvaluationArtifact);
  if (sha256(sourceBytes) !== args.staleEvaluationSha256.toLowerCase()) {
    throw new Error("serious stale application evaluation SHA-256 does not match the supplied file");
  }
  const sourceDocument = parseSanitizedEvidence(sourceBytes, "serious stale application evaluation");
  validateStaleEvaluationDocument(sourceDocument, productVersion, gitCommit);
  const document = retainJsonEvidence({
    sourcePath: args.staleEvaluationArtifact,
    destinationPath,
    expectedSha256: args.staleEvaluationSha256,
    label: "serious stale application evaluation",
    resume,
    allowExistingIdentical,
  });
  validateStaleEvaluationDocument(document, productVersion, gitCommit);
  return {
    status: "measured",
    count: document.seriousStaleApplications,
    scenariosEvaluated: document.scenariosEvaluated,
    evaluatedAt: document.evaluatedAt,
    evidenceSha256: args.staleEvaluationSha256.toLowerCase(),
    evidencePath: relative(dirname(outputPath), destinationPath),
  };
}

function validateStaleEvaluationDocument(document, productVersion, gitCommit) {
  if (
    document.schemaVersion !== 1 ||
    document.evidenceClass !== "serious_stale_application_evaluation" ||
    document.product !== "PreviouslyOn" ||
    document.productVersion !== productVersion ||
    document.gitCommit !== gitCommit ||
    document.status !== "measured" ||
    !Number.isInteger(document.scenariosEvaluated) ||
    document.scenariosEvaluated < 1 ||
    !Number.isInteger(document.seriousStaleApplications) ||
    document.seriousStaleApplications < 0 ||
    !Number.isFinite(Date.parse(document.evaluatedAt))
  ) {
    throw new Error("serious stale application evaluator artifact has an invalid or mismatched contract");
  }
}

function parseArgs(raw) {
  const args = {
    dryRun: false,
    keepTemporary: false,
    resume: false,
    finalizeStaleEvaluation: false,
    timeoutMs: 10 * 60_000,
  };
  const names = {
    "--latest-bin": "latestBin",
    "--previous-bin": "previousBin",
    "--previously-bin": "previouslyBin",
    "--mapped-artifact": "mappedArtifact",
    "--codex-home": "codexHome",
    "--model": "model",
    "--reasoning-effort": "reasoningEffort",
    "--output": "output",
    "--bundle": "bundle",
    "--evidence-dir": "evidenceDir",
    "--confirm": "confirm",
    "--timeout-seconds": "timeoutSeconds",
    "--codex-app-current-build": "codexAppCurrentBuild",
    "--codex-app-current-evidence": "codexAppCurrentEvidence",
    "--codex-app-current-evidence-sha256": "codexAppCurrentEvidenceSha256",
    "--codex-app-previous-build": "codexAppPreviousBuild",
    "--codex-app-previous-evidence": "codexAppPreviousEvidence",
    "--codex-app-previous-evidence-sha256": "codexAppPreviousEvidenceSha256",
    "--codex-app-previous-unavailable-reason": "codexAppPreviousUnavailableReason",
    "--codex-app-previous-checked-at": "codexAppPreviousCheckedAt",
    "--stale-evaluation-artifact": "staleEvaluationArtifact",
    "--stale-evaluation-sha256": "staleEvaluationSha256",
  };
  for (let index = 0; index < raw.length; index += 1) {
    const token = raw[index];
    if (token === "--dry-run") args.dryRun = true;
    else if (token === "--keep-temporary") args.keepTemporary = true;
    else if (token === "--resume") args.resume = true;
    else if (token === "--finalize-stale-evaluation") args.finalizeStaleEvaluation = true;
    else if (names[token]) args[names[token]] = raw[++index];
    else throw new Error(`unknown argument ${token}`);
  }
  if (args.timeoutSeconds) {
    const seconds = Number(args.timeoutSeconds);
    if (!Number.isInteger(seconds) || seconds < 30) throw new Error("--timeout-seconds must be an integer >= 30");
    args.timeoutMs = seconds * 1_000;
  }
  return args;
}

function requireBoundCodexExecution(args, contract) {
  if (
    args.model !== contract.execution.model ||
    args.reasoningEffort !== contract.execution.reasoningEffort ||
    args.timeoutMs !== contract.execution.timeoutSeconds * 1_000
  ) {
    throw new Error(
      `live execution must use contract-bound ${contract.execution.model}, ${contract.execution.reasoningEffort} reasoning, and ${contract.execution.timeoutSeconds}s timeout`,
    );
  }
}

function retainJsonEvidence({
  sourcePath,
  destinationPath,
  expectedSha256,
  label,
  resume,
  allowExistingIdentical = false,
}) {
  requireRegularAbsoluteFile(sourcePath, label);
  const sourceBytes = readFileSync(sourcePath);
  if (sha256(sourceBytes) !== expectedSha256.toLowerCase()) {
    throw new Error(`${label} SHA-256 does not match the supplied file`);
  }
  const document = parseSanitizedEvidence(sourceBytes, label);
  if (resume) {
    validateBoundEvidenceFile(destinationPath, expectedSha256, label);
  } else if (allowExistingIdentical && existsSync(destinationPath)) {
    validateBoundEvidenceFile(destinationPath, expectedSha256, label);
  } else {
    writePrivateBytes(destinationPath, sourceBytes);
  }
  return document;
}

function validateArtifactEvidenceBindings(codexApp, seriousStaleApplications, outputPath) {
  for (const [label, run] of [
    ["current Codex App evidence", codexApp?.current],
    ["previous Codex App evidence", codexApp?.previous],
  ]) {
    if (run?.status === "passed" || run?.status === "degraded") {
      const path = resolveBoundEvidencePath(outputPath, run.evidencePath, label);
      const document = validateBoundEvidenceFile(path, run.evidenceSha256, label);
      if (document.build !== run.build || document.status !== run.status) {
        throw new Error(`${label} identity does not match its retained file`);
      }
    }
  }
  if (seriousStaleApplications?.status === "measured") {
    const label = "serious stale application evaluation";
    const path = resolveBoundEvidencePath(outputPath, seriousStaleApplications.evidencePath, label);
    const document = validateBoundEvidenceFile(path, seriousStaleApplications.evidenceSha256, label);
    if (
      document.status !== "measured" ||
      document.seriousStaleApplications !== seriousStaleApplications.count ||
      document.scenariosEvaluated !== seriousStaleApplications.scenariosEvaluated
    ) {
      throw new Error(`${label} identity does not match its retained file`);
    }
  }
}

function validateBoundEvidenceFile(path, expectedSha256, label) {
  if (
    !existsSync(path) ||
    !lstatSync(path).isFile() ||
    lstatSync(path).isSymbolicLink() ||
    !/^[0-9a-f]{64}$/i.test(expectedSha256 ?? "")
  ) {
    throw new Error(`${label} retained file is missing or invalid`);
  }
  const bytes = readFileSync(path);
  if (sha256(bytes) !== expectedSha256.toLowerCase()) {
    throw new Error(`${label} retained file hash changed`);
  }
  return parseSanitizedEvidence(bytes, label);
}

function resolveBoundEvidencePath(outputPath, evidencePath, label) {
  if (typeof evidencePath !== "string" || evidencePath.trim() === "") {
    throw new Error(`${label} omitted its retained evidence path`);
  }
  const outputDirectory = dirname(resolve(outputPath));
  const path = resolve(outputDirectory, evidencePath);
  if (path === outputDirectory || !path.startsWith(`${outputDirectory}/`)) {
    throw new Error(`${label} retained evidence path escapes the bundle`);
  }
  return path;
}

function parseSanitizedEvidence(bytes, label) {
  let document;
  try {
    document = JSON.parse(bytes.toString("utf8"));
  } catch {
    throw new Error(`${label} must be valid JSON`);
  }
  assertSanitizedEvidence(document, label);
  return document;
}

function assertSanitizedEvidence(value, label, keyPath = "$") {
  if (Array.isArray(value)) {
    value.forEach((item, index) => assertSanitizedEvidence(item, label, `${keyPath}[${index}]`));
    return;
  }
  if (!value || typeof value !== "object") return;
  for (const [key, item] of Object.entries(value)) {
    const normalizedKey = key.toLowerCase().replaceAll(/[^a-z0-9]/g, "");
    if (["prompt", "tooloutput", "sourcecode", "credential", "credentials", "repositorypath", "rawjsonl"].includes(normalizedKey)) {
      throw new Error(`${label} retained forbidden raw field ${keyPath}.${key}`);
    }
    if (typeof item === "string") {
      if (
        item.startsWith("/Users/") ||
        item.startsWith("/private/") ||
        item.includes("-----BEGIN PRIVATE KEY-----") ||
        /authorization\s*:\s*bearer/i.test(item)
      ) {
        throw new Error(`${label} retained forbidden raw data at ${keyPath}.${key}`);
      }
    } else {
      assertSanitizedEvidence(item, label, `${keyPath}.${key}`);
    }
  }
}

function requireRegularAbsoluteFile(path, label) {
  if (
    !path ||
    !isAbsolute(path) ||
    !existsSync(path) ||
    !lstatSync(path).isFile() ||
    lstatSync(path).isSymbolicLink()
  ) {
    throw new Error(`${label} must be an existing absolute regular file, not a symlink`);
  }
}

function writePrivateBytes(path, bytes) {
  const directory = dirname(path);
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const temporary = join(directory, `.${basename(path)}.${process.pid}.tmp`);
  const handle = openSync(temporary, "wx", 0o600);
  try {
    writeFileSync(handle, bytes);
    fsyncSync(handle);
  } finally {
    closeSync(handle);
  }
  renameSync(temporary, path);
  chmodSync(path, 0o600);
  const directoryHandle = openSync(directory, "r");
  try {
    fsyncSync(directoryHandle);
  } finally {
    closeSync(directoryHandle);
  }
}

function releaseIneligibilityReason(artifact) {
  if (artifact.codexApp?.current?.status !== "passed") {
    return "current Codex App evidence is degraded or unverified";
  }
  const stale = artifact.releaseEligibility?.seriousStaleApplications;
  if (stale?.status !== "measured") {
    return "serious stale applications remain unmeasured by a separately retained evaluator artifact";
  }
  if (stale.count !== 0) return "the retained evaluator recorded one or more serious stale applications";
  if (artifact.releaseEligibility?.dataLossEvents !== 0) return "one or more data-loss diagnostics were observed";
  return "one or more required release evidence bindings are incomplete";
}

function requireExecutable(path, label) {
  if (!path || !isAbsolute(path) || !existsSync(path)) throw new Error(`${label} must be an existing absolute path`);
  accessSync(path, fsConstants.X_OK);
}

function checked(command, args, options = {}) {
  const result = spawnSyncLike(command, args, options);
  if (result.code !== 0) throw new Error(`${basename(command)} exited ${result.code}: ${tail(result.stderr)}`);
  return result;
}

function checkedText(command, args, options = {}) {
  return checked(command, args, options).stdout;
}

function spawnSyncLike(command, args, options) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env,
    encoding: "utf8",
    maxBuffer: MAX_CAPTURE_BYTES,
  });
  if (result.error) throw result.error;
  return { code: result.status ?? 1, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

async function runCaptured(command, args, options = {}) {
  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(command, args, {
      cwd: options.cwd,
      env: options.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = Buffer.alloc(0);
    let stderr = Buffer.alloc(0);
    let terminationReason = null;
    let forceKill = null;
    const terminate = (reason) => {
      if (terminationReason) return;
      terminationReason = reason;
      child.kill("SIGTERM");
      forceKill = setTimeout(() => child.kill("SIGKILL"), 5_000);
      forceKill.unref();
    };
    const append = (current, chunk) => {
      const next = Buffer.concat([current, chunk]);
      if (next.length > MAX_CAPTURE_BYTES) {
        terminate(`${basename(command)} exceeded ${MAX_CAPTURE_BYTES} captured bytes`);
      }
      return next;
    };
    child.stdout.on("data", (chunk) => { stdout = append(stdout, chunk); });
    child.stderr.on("data", (chunk) => { stderr = append(stderr, chunk); });
    const timeout = setTimeout(() => {
      terminate(`${basename(command)} timed out after ${options.timeoutMs ?? 30_000}ms`);
    }, options.timeoutMs ?? 30_000);
    child.on("error", (error) => {
      clearTimeout(timeout);
      if (forceKill) clearTimeout(forceKill);
      rejectPromise(error);
    });
    child.on("close", (code) => {
      clearTimeout(timeout);
      if (forceKill) clearTimeout(forceKill);
      if (terminationReason) rejectPromise(new Error(terminationReason));
      else resolvePromise({ code: code ?? 1, stdout: stdout.toString("utf8"), stderr: stderr.toString("utf8") });
    });
  });
}

function render(template, replacements) {
  return template.replace(/\{\{([A-Z_]+)\}\}/g, (_, key) => {
    if (!(key in replacements)) throw new Error(`template uses unknown placeholder ${key}`);
    return replacements[key];
  });
}

function stableStringify(value) {
  if (Array.isArray(value)) return `[${value.map(stableStringify).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, value) {
  const directory = dirname(path);
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const temporary = join(directory, `.${basename(path)}.${process.pid}.tmp`);
  const handle = openSync(temporary, "w", 0o600);
  try {
    writeFileSync(handle, `${JSON.stringify(value, null, 2)}\n`);
    fsyncSync(handle);
  } finally {
    closeSync(handle);
  }
  renameSync(temporary, path);
  chmodSync(path, 0o600);
  const directoryHandle = openSync(directory, "r");
  try {
    fsyncSync(directoryHandle);
  } finally {
    closeSync(directoryHandle);
  }
}

function writePrivate(path, contents) {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, contents, { mode: 0o600 });
  chmodSync(path, 0o600);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function countDataLoss(text) {
  return (text.match(/DATA LOSS/gi) ?? []).length;
}

function tail(text, length = 500) {
  return text.replace(/\s+/g, " ").slice(-length);
}

function safeError(error) {
  return sanitizeDiagnostic(tail(error instanceof Error ? error.message : String(error)));
}

function sanitizeDiagnostic(value) {
  return value
    .replace(/\b(?:sk|sess|rk|pk)-[A-Za-z0-9_-]{8,}\b/g, "[REDACTED]")
    .replace(/\bBearer\s+[^\s]+/gi, "Bearer [REDACTED]")
    .replace(/([A-Z][A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD))\s*=\s*[^\s]+/g, "$1=[REDACTED]")
    .replace(/([a-z][a-z0-9+.-]*:\/\/[^\s:@/]+:)[^\s@/]+@/gi, "$1[REDACTED]@");
}

function compareSemver(left, right) {
  const a = left.split(".").map(Number);
  const b = right.split(".").map(Number);
  return a[0] - b[0] || a[1] - b[1] || a[2] - b[2];
}

function camelToKebab(value) {
  return value.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(SCRIPT_PATH)) {
  main().catch((error) => {
    process.stderr.write(`error: ${safeError(error)}\n`);
    process.exitCode = 1;
  });
}
