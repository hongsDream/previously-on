#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, lstatSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const [artifactPath, ...rawArgs] = process.argv.slice(2);
if (!artifactPath) fail('usage: validate-live-compatibility.mjs ARTIFACT --commit SHA --product-version VERSION [--resolve-current-codex]');
const artifactAbsolutePath = resolve(artifactPath);
const artifactDirectory = dirname(artifactAbsolutePath);

const args = new Map();
for (let index = 0; index < rawArgs.length; index += 1) {
  const key = rawArgs[index];
  if (key === '--resolve-current-codex') {
    args.set(key, true);
  } else {
    args.set(key, rawArgs[++index]);
  }
}
const expectedCommit = requiredArg('--commit');
const expectedProductVersion = requiredArg('--product-version');
if (!/^[0-9a-f]{40}$/i.test(expectedCommit)) fail('--commit must be a full 40-character Git SHA');

const artifact = JSON.parse(readFileSync(artifactAbsolutePath, 'utf8'));
const liveContract = JSON.parse(readFileSync(`${root}/fixtures/compatibility/live-workflow-contract.json`, 'utf8'));
const executionFields = ['model', 'reasoningEffort', 'sandbox', 'strictConfig', 'timeoutSeconds'];
const reconstructionFields = ['userPrompt', 'assistantFinal', 'fileChangeTool', 'testCommand', 'modelIdentity', 'stableSourceIds'];
assertEqual(artifact.schemaVersion, 1, 'schemaVersion');
assertEqual(artifact.evidenceClass, 'live_codex_workflow_matrix', 'evidenceClass');
assertEqual(artifact.product, 'PreviouslyOn', 'product');
assertEqual(artifact.productVersion, expectedProductVersion, 'productVersion');
assertEqual(artifact.gitCommit, expectedCommit, 'gitCommit');
assertEqual(artifact.supportMode, 'explicit_run_and_import', 'supportMode');
assertEqual(artifact.runner?.os, 'macOS', 'runner.os');
assertEqual(artifact.runner?.arch, 'arm64', 'runner.arch');
for (const field of executionFields) {
  assertEqual(artifact.codexExecution?.[field], liveContract.execution?.[field], `codexExecution.${field}`);
}
if (!Number.isFinite(Date.parse(artifact.generatedAt))) fail('generatedAt must be an ISO-8601 timestamp');

assertEqual(artifact.releaseEligibility?.eligible, true, 'releaseEligibility.eligible');
assertEqual(artifact.releaseEligibility?.dataLossEvents, 0, 'releaseEligibility.dataLossEvents');
const staleEvaluation = artifact.releaseEligibility?.seriousStaleApplications;
assertEqual(staleEvaluation?.status, 'measured', 'releaseEligibility.seriousStaleApplications.status');
assertEqual(staleEvaluation?.count, 0, 'releaseEligibility.seriousStaleApplications.count');
if (!Number.isInteger(staleEvaluation?.scenariosEvaluated) || staleEvaluation.scenariosEvaluated < 1) {
  fail('releaseEligibility.seriousStaleApplications.scenariosEvaluated must be positive');
}
if (!Number.isFinite(Date.parse(staleEvaluation?.evaluatedAt))) {
  fail('releaseEligibility.seriousStaleApplications.evaluatedAt must be ISO-8601');
}
assertEqual(artifact.liveCodexWorkflowMatrix?.status, 'complete', 'liveCodexWorkflowMatrix.status');
assertEqual(artifact.liveCodexWorkflowMatrix?.completedRuns, 60, 'liveCodexWorkflowMatrix.completedRuns');
assertEqual(artifact.transparentCaptureReleaseGate?.eligible, true, 'transparentCaptureReleaseGate.eligible');
for (const field of ['fixtureContractSha256', 'scenarioMatrixSha256', 'previouslyBinarySha256']) {
  if (!/^[0-9a-f]{64}$/i.test(artifact[field] ?? '')) fail(`${field} must be SHA-256`);
}
assertEqual(
  artifact.fixtureContractSha256,
  sha256(readFileSync(`${root}/fixtures/compatibility/live-workflow-contract.json`)),
  'fixtureContractSha256',
);
assertEqual(
  artifact.scenarioMatrixSha256,
  sha256(readFileSync(`${root}/fixtures/compatibility/scenarios.json`)),
  'scenarioMatrixSha256',
);
assertEqual(artifact.localMappedRegression?.status, 'separate_artifact_passed', 'localMappedRegression.status');
if (!/^[0-9a-f]{64}$/i.test(artifact.localMappedRegression?.artifactSha256 ?? '')) {
  fail('localMappedRegression.artifactSha256 must be SHA-256');
}
assertEqual(artifact.localMappedRegression?.gitCommit, expectedCommit, 'localMappedRegression.gitCommit');
assertEqual(artifact.localMappedRegression?.scenarioCount, 30, 'localMappedRegression.scenarioCount');
const observedEvidencePaths = new Set();
const mappedEvidence = readBoundEvidence(
  artifact.localMappedRegression?.evidencePath,
  artifact.localMappedRegression?.artifactSha256,
  'localMappedRegression',
);
assertEqual(mappedEvidence.gitCommit, expectedCommit, 'mapped evidence gitCommit');
assertEqual(mappedEvidence.localMappedRegression?.scenarioCount, 30, 'mapped evidence scenarioCount');

const matrix = JSON.parse(readFileSync(`${root}/fixtures/compatibility/scenarios.json`, 'utf8'));
const expectedScenarios = new Map(matrix.scenarios.map((scenario) => [scenario.id, scenario]));
if (expectedScenarios.size !== 30) fail('repository compatibility fixture must contain exactly 30 unique scenarios');

const cliRuns = artifact.codexCli?.runs;
if (!Array.isArray(cliRuns) || cliRuns.length !== 2) fail('codexCli.runs must contain latest and previous runs');
const roles = new Map(cliRuns.map((run) => [run.role, run]));
if (roles.size !== 2 || !roles.has('latest') || !roles.has('previous')) fail('codexCli.runs roles must be latest and previous');
if (roles.get('latest').version === roles.get('previous').version) fail('latest and previous Codex versions must differ');
validateMappedRows(mappedEvidence, roles);

if (args.get('--resolve-current-codex')) {
  const { latest, previous } = resolveStableCodexVersions();
  assertEqual(roles.get('latest').version, latest, 'codexCli latest version');
  assertEqual(roles.get('previous').version, previous, 'codexCli previous version');
}

for (const role of ['latest', 'previous']) validateCliRun(roles.get(role), role);
validateAppRun(artifact.codexApp?.current, 'codexApp.current', false);
validateAppRun(artifact.codexApp?.previous, 'codexApp.previous', true);
const staleEvidence = readBoundEvidence(
  staleEvaluation.evidencePath,
  staleEvaluation.evidenceSha256,
  'releaseEligibility.seriousStaleApplications',
);
assertEqual(staleEvidence.schemaVersion, 1, 'stale evidence schemaVersion');
assertEqual(staleEvidence.evidenceClass, 'serious_stale_application_evaluation', 'stale evidence evidenceClass');
assertEqual(staleEvidence.product, 'PreviouslyOn', 'stale evidence product');
assertEqual(staleEvidence.productVersion, expectedProductVersion, 'stale evidence productVersion');
assertEqual(staleEvidence.gitCommit, expectedCommit, 'stale evidence gitCommit');
assertEqual(staleEvidence.status, 'measured', 'stale evidence status');
assertEqual(staleEvidence.seriousStaleApplications, 0, 'stale evidence seriousStaleApplications');
assertEqual(staleEvidence.scenariosEvaluated, staleEvaluation.scenariosEvaluated, 'stale evidence scenariosEvaluated');
assertEqual(staleEvidence.evaluatedAt, staleEvaluation.evaluatedAt, 'stale evidence evaluatedAt');
assertSanitizedEvidence(staleEvidence, 'stale evidence');

process.stdout.write(`eligible live compatibility evidence verified: ${artifactPath}\n`);

function validateCliRun(run, role) {
  if (!/^\d+\.\d+\.\d+$/.test(run?.version ?? '')) fail(`${role} Codex version must be stable semver`);
  if (!/^[0-9a-f]{64}$/i.test(run?.binarySha256 ?? '')) fail(`${role} binarySha256 is required`);
  if (!Array.isArray(run.scenarios) || run.scenarios.length !== 30) fail(`${role} must contain 30 live scenarios`);
  const seen = new Set();
  for (const scenario of run.scenarios) {
    if (!expectedScenarios.has(scenario.id)) fail(`${role} contains unknown scenario ${scenario.id}`);
    if (!seen.add(scenario.id)) fail(`${role} contains duplicate scenario ${scenario.id}`);
    assertEqual(scenario.category, expectedScenarios.get(scenario.id).category, `${role}/${scenario.id} category`);
    assertEqual(scenario.status, 'passed', `${role}/${scenario.id} status`);
    for (const field of executionFields) {
      assertEqual(
        scenario.codexExecution?.[field],
        artifact.codexExecution[field],
        `${role}/${scenario.id} codexExecution.${field}`,
      );
    }
    for (const field of reconstructionFields) {
      assertEqual(scenario.reconstruction?.[field], true, `${role}/${scenario.id} reconstruction.${field}`);
    }
    const mapped = scenario.scenarioAssertion;
    assertEqual(mapped?.status, 'passed', `${role}/${scenario.id} scenarioAssertion.status`);
    const definition = expectedScenarios.get(scenario.id);
    assertEqual(mapped?.testTarget, definition.testTarget, `${role}/${scenario.id} scenarioAssertion.testTarget`);
    assertEqual(mapped?.testFilter, definition.testFilter, `${role}/${scenario.id} scenarioAssertion.testFilter`);
    assertEqual(mapped?.expectation, definition.expectation, `${role}/${scenario.id} scenarioAssertion.expectation`);
    assertEqual(
      mapped?.mappedArtifactSha256,
      artifact.localMappedRegression.artifactSha256,
      `${role}/${scenario.id} scenarioAssertion.mappedArtifactSha256`,
    );
    if (!/^[0-9a-f]{64}$/i.test(scenario.evidenceSha256 ?? '')) fail(`${role}/${scenario.id} must include evidenceSha256`);
    const evidence = readBoundEvidence(
      scenario.evidencePath,
      scenario.evidenceSha256,
      `${role}/${scenario.id}`,
    );
    assertEqual(evidence.schemaVersion, 1, `${role}/${scenario.id} evidence schemaVersion`);
    assertEqual(evidence.role, role, `${role}/${scenario.id} evidence role`);
    assertEqual(evidence.codexVersion, run.version, `${role}/${scenario.id} evidence codexVersion`);
    for (const field of executionFields) {
      assertEqual(
        evidence.codexExecution?.[field],
        artifact.codexExecution[field],
        `${role}/${scenario.id} evidence codexExecution.${field}`,
      );
    }
    assertEqual(evidence.scenario?.id, scenario.id, `${role}/${scenario.id} evidence scenario.id`);
    assertEqual(
      evidence.scenario?.category,
      scenario.category,
      `${role}/${scenario.id} evidence scenario.category`,
    );
    assertEqual(evidence.scenarioAssertion?.status, 'passed', `${role}/${scenario.id} evidence scenarioAssertion.status`);
    assertEqual(evidence.verification?.passed, true, `${role}/${scenario.id} evidence verification.passed`);
    for (const field of reconstructionFields) {
      assertEqual(
        evidence.verification?.reconstruction?.[field],
        true,
        `${role}/${scenario.id} evidence verification.reconstruction.${field}`,
      );
    }
    assertSanitizedEvidence(evidence, `${role}/${scenario.id}`);
  }
  if (seen.size !== expectedScenarios.size) fail(`${role} does not cover every required scenario`);
}

function readBoundEvidence(relativePath, expectedSha256, label) {
  if (typeof relativePath !== 'string' || relativePath.trim() === '') {
    fail(`${label}.evidencePath is required`);
  }
  const normalized = relativePath.replaceAll('\\', '/');
  if (normalized.startsWith('/') || normalized.split('/').includes('..')) {
    fail(`${label}.evidencePath escapes the evidence bundle`);
  }
  const path = resolve(artifactDirectory, relativePath);
  if (path === artifactDirectory || !path.startsWith(`${artifactDirectory}/`)) {
    fail(`${label}.evidencePath escapes the evidence bundle`);
  }
  if (!observedEvidencePaths.add(path)) {
    fail(`${label}.evidencePath is reused by another evidence entry`);
  }
  if (!existsSync(path)) fail(`${label}.evidencePath is missing from the evidence bundle`);
  const metadata = lstatSync(path);
  if (!metadata.isFile() || metadata.isSymbolicLink()) fail(`${label}.evidencePath must be a regular non-symlink file`);
  const bytes = readFileSync(path);
  assertEqual(sha256(bytes), expectedSha256, `${label}.evidenceSha256`);
  try {
    return JSON.parse(bytes.toString('utf8'));
  } catch {
    fail(`${label}.evidencePath is not valid JSON`);
  }
}

function assertSanitizedEvidence(value, label, keyPath = '$') {
  if (Array.isArray(value)) {
    value.forEach((item, index) => assertSanitizedEvidence(item, label, `${keyPath}[${index}]`));
    return;
  }
  if (!value || typeof value !== 'object') return;
  for (const [key, item] of Object.entries(value)) {
    const normalizedKey = key.toLowerCase().replaceAll(/[^a-z0-9]/g, '');
    if (['prompt', 'tooloutput', 'sourcecode', 'credential', 'credentials', 'repositorypath', 'rawjsonl'].includes(normalizedKey)) {
      fail(`${label} retained forbidden raw field ${keyPath}.${key}`);
    }
    if (typeof item === 'string') {
      if (
        item.startsWith('/Users/') ||
        item.startsWith('/private/') ||
        item.includes('-----BEGIN PRIVATE KEY-----') ||
        /authorization\s*:\s*bearer/i.test(item)
      ) {
        fail(`${label} retained forbidden raw data at ${keyPath}.${key}`);
      }
    } else {
      assertSanitizedEvidence(item, label, `${keyPath}.${key}`);
    }
  }
}

function validateAppRun(run, label, allowUnavailable) {
  if (allowUnavailable && run?.status === 'unavailable') {
    if (!run.reason || !Number.isFinite(Date.parse(run.checkedAt))) fail(`${label} unavailable result requires reason and checkedAt`);
    return;
  }
  assertEqual(run?.status, 'passed', `${label}.status`);
  if (typeof run?.build !== 'string' || run.build.trim() === '') fail(`${label}.build is required`);
  if (!/^[0-9a-f]{64}$/i.test(run.evidenceSha256 ?? '')) fail(`${label}.evidenceSha256 is required`);
  const evidence = readBoundEvidence(run.evidencePath, run.evidenceSha256, label);
  assertEqual(evidence.schemaVersion, 1, `${label} evidence schemaVersion`);
  assertEqual(evidence.evidenceClass, 'codex_app_verification', `${label} evidence evidenceClass`);
  assertEqual(evidence.product, 'Codex App', `${label} evidence product`);
  assertEqual(evidence.status, 'passed', `${label} evidence status`);
  assertEqual(evidence.build, run.build, `${label} evidence build`);
  if (!Number.isFinite(Date.parse(evidence.checkedAt))) fail(`${label} evidence checkedAt must be ISO-8601`);
  assertSanitizedEvidence(evidence, label);
}

function validateMappedRows(mappedEvidence, roles) {
  const runs = mappedEvidence.localMappedRegression?.runs;
  if (!Array.isArray(runs) || runs.length !== 2) fail('mapped evidence must contain two actual regression runs');
  const byVersion = new Map(runs.map((run) => [run.codexVersionSlot, run]));
  for (const role of ['latest', 'previous']) {
    const version = roles.get(role).version;
    const run = byVersion.get(version);
    if (!run || !Array.isArray(run.scenarioResults) || run.scenarioResults.length !== expectedScenarios.size) {
      fail(`mapped evidence omitted actual scenario rows for ${role} Codex ${version}`);
    }
    const seen = new Set();
    for (const result of run.scenarioResults) {
      const expected = expectedScenarios.get(result.id);
      if (!expected || !seen.add(result.id)) fail(`mapped evidence contains unknown or duplicate row ${result.id}`);
      assertEqual(result.testTarget, expected.testTarget, `mapped ${role}/${result.id} testTarget`);
      assertEqual(result.testFilter, expected.testFilter, `mapped ${role}/${result.id} testFilter`);
      assertEqual(result.expectation, expected.expectation, `mapped ${role}/${result.id} expectation`);
      assertEqual(result.status, 'passed', `mapped ${role}/${result.id} status`);
      assertEqual(result.exitCode, 0, `mapped ${role}/${result.id} exitCode`);
    }
  }
}

function resolveStableCodexVersions() {
  const versions = JSON.parse(execFileSync('npm', ['view', '@openai/codex', 'versions', '--json'], { encoding: 'utf8' }));
  const stable = versions.filter((version) => /^\d+\.\d+\.\d+$/.test(version)).sort(compareSemver);
  if (stable.length < 2) fail('npm returned fewer than two stable Codex versions');
  return { latest: stable.at(-1), previous: stable.at(-2) };
}

function compareSemver(left, right) {
  const a = left.split('.').map(Number);
  const b = right.split('.').map(Number);
  return a[0] - b[0] || a[1] - b[1] || a[2] - b[2];
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

function requiredArg(name) {
  const value = args.get(name);
  if (!value) fail(`${name} is required`);
  return value;
}

function assertEqual(actual, expected, label) {
  if (actual !== expected) fail(`${label} must be ${JSON.stringify(expected)}, found ${JSON.stringify(actual)}`);
}

function fail(message) {
  process.stderr.write(`error: ${message}\n`);
  process.exit(1);
}
