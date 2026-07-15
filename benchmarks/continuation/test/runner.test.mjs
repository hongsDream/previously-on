import assert from 'node:assert/strict';
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  completedArmKeys,
  createDryRunPlan,
  benchmarkHarnessInputPaths,
  buildCampaignLock,
  inspectProviderBinary,
  restoreUsageState,
  runCampaign,
  shouldPauseForUsage,
  syntheticTemplateDigests,
  validateCalibrationEvidence,
  validateProductSourceCheckpointBinding,
} from '../runner.mjs';
import { FakeProvider, rateLimitSnapshot } from '../src/fake-provider.mjs';
import { readJson, readJsonLines, sha256, stableStringify, writeJsonAtomic } from '../src/io.mjs';
import { buildBaseMatrix, loadFixtureSet } from '../src/validation.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const MANIFEST = readJson(join(ROOT, 'manifest.v1.json'));
const ALL_FIXTURES = loadFixtureSet(join(ROOT, 'fixtures'));
const FIXTURE = ALL_FIXTURES.find((entry) => entry.fixture.id === 'synthetic-config-guard');
const RESULT_SCHEMA = readJson(join(ROOT, 'schemas/result.v1.schema.json'));
const CONTROL_SCHEMA = readJson(join(ROOT, 'schemas/control.v1.schema.json'));

async function temporaryCase(t) {
  const root = await mkdtemp(join(tmpdir(), 'previously-on-continuation-runner-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  return {
    root,
    resultsPath: join(root, 'results.v1.jsonl'),
    controlPath: join(root, 'control.v1.jsonl'),
    benchmarkRoot: join(root, 'benchmark'),
  };
}

function campaignLock() {
  return {
    schemaVersion: 1,
    benchmarkId: MANIFEST.benchmarkId,
    prerequisiteMergeSha: MANIFEST.prerequisite.mergeSha,
    fixtureSetSha256: MANIFEST.fixtureSet.expectedSha256,
    manifestSha256: sha256(stableStringify(MANIFEST)),
    syntheticTemplateSha256: syntheticTemplateDigests(ALL_FIXTURES),
    models: MANIFEST.execution.models.map((model) => ({ requested: model.id, actualSnapshotId: `${model.id}-fake-snapshot` })),
  };
}

function expectedFinal() {
  return {
    schemaVersion: 1,
    scenarioId: FIXTURE.fixture.id,
    completionMarker: FIXTURE.fixture.finalChallenge.completionMarker,
    ...structuredClone(FIXTURE.fixture.expectedState),
  };
}

function twoArms() {
  return buildBaseMatrix(MANIFEST, ALL_FIXTURES)
    .filter((arm) => arm.model === 'gpt-5.5' && arm.scenario === FIXTURE.fixture.id && arm.compaction === 0 && arm.repetition === 1);
}

function productArm() {
  const native = twoArms().find((arm) => arm.strategy === 'native_handoff');
  const product = { ...native, strategy: 'verified_context_pack_contracts' };
  product.key = [
    product.model,
    product.scenario,
    product.strategy,
    product.compaction,
    product.repetition,
    product.fixtureSha256,
  ].join('/');
  return product;
}

function successfulWorkspace(paths) {
  return {
    prepareArmWorkspace: async ({ armKey }) => ({
      repositoryRoot: paths.root,
      workspaceId: sha256(armKey),
      headSha: 'b'.repeat(40),
      templateSha256: syntheticTemplateDigests(ALL_FIXTURES)[FIXTURE.fixture.id],
    }),
    inspectWorkspaceChanges: async () => ({
      changedFiles: [],
      unexpectedFiles: [],
      changeDigestSha256: 'c'.repeat(64),
    }),
    runFixtureTest: async () => ({
      argv: ['fake-fixture-test'],
      passed: true,
      declaredPassed: true,
      declaredTestCount: 1,
      requiredTestCount: 1,
      executionCountPassed: true,
      oracle: { passed: true, violatedInvariantIds: [] },
      exitCode: 0,
      signal: null,
      timedOut: false,
      durationMs: 1,
      stdout: '',
      stderr: '',
      stdoutBytes: 0,
      stderrBytes: 0,
      outputTruncated: false,
    }),
  };
}

function productContextFor(source, overrides = {}) {
  return {
    schemaVersion: 1,
    binding: {
      sourceKey: source.sourceKey,
      sourceCompaction: source.compaction,
      sourceThreadId: source.snapshotThreadId,
      sourceSnapshotSha256: source.sourceSnapshotSha256,
      ...overrides,
    },
    materialization: { durationMs: 30 },
    sha256: 'a'.repeat(64),
  };
}

test('dry-run fixes the base matrix at 864 paid-arm-free records', () => {
  const plan = createDryRunPlan(MANIFEST, ALL_FIXTURES);
  assert.equal(plan.measuredArmCount, 864);
  assert.equal(plan.pairedSourceCount, 432);
  assert.equal(plan.paidTurnsExecuted, 0);
  assert.equal(plan.productArmsIncluded, false);
});

test('campaign harness binding includes every synthetic repository template byte', () => {
  const inputs = new Set(benchmarkHarnessInputPaths());
  for (const { fixture } of ALL_FIXTURES.filter(({ fixture }) => fixture.kind === 'synthetic')) {
    const prefix = `repositories/${fixture.id}/`;
    assert.equal(
      [...inputs].some((path) => path.startsWith(prefix)),
      true,
      `${fixture.id} template is absent from the campaign harness binding`,
    );
  }
  assert.equal(inputs.has('repositories/synthetic-config-guard/src/config.ts'), true);
  assert.equal(inputs.has('repositories/synthetic-parser-rename/Cargo.lock'), true);
  const digests = syntheticTemplateDigests(ALL_FIXTURES);
  assert.equal(Object.keys(digests).length, 4);
  assert.equal(digests['synthetic-config-guard'].length, 64);
});

test('provider provenance records local code-signature status but rejects non-Codex version output', async (t) => {
  const paths = await temporaryCase(t);
  const codex = join(paths.root, 'codex');
  await writeFile(codex, '#!/bin/sh\nprintf "codex-cli 1.2.3\\n"\n');
  await chmod(codex, 0o700);
  const inspected = inspectProviderBinary(codex);
  assert.equal(inspected.providerVersion, 'codex-cli 1.2.3');
  assert.ok(['valid', 'invalid', 'unavailable'].includes(inspected.codeSignature.status));

  const impostor = join(paths.root, 'impostor');
  await writeFile(impostor, '#!/bin/sh\nprintf "unrelated-tool 1.2.3\\n"\n');
  await chmod(impostor, 0o700);
  assert.throws(() => inspectProviderBinary(impostor), /unexpected Codex version string/);
});

test('product context source checkpoint binding validation is exact and fail-closed', () => {
  const input = {
    arm: { key: 'product-arm-key' },
    source: {
      sourceKey: 'gpt-5.5/synthetic-config-guard/1',
      compaction: 4,
      snapshotThreadId: 'source-thread-1',
      sourceSnapshotSha256: 'a'.repeat(64),
    },
  };
  const product = {
    binding: {
      sourceKey: input.source.sourceKey,
      sourceCompaction: input.source.compaction,
      sourceThreadId: input.source.snapshotThreadId,
      sourceSnapshotSha256: input.source.sourceSnapshotSha256,
    },
  };
  assert.equal(validateProductSourceCheckpointBinding(product, input), product);
  for (const [field, replacement] of [
    ['sourceKey', 'gpt-5.5/synthetic-config-guard/2'],
    ['sourceCompaction', 5],
    ['sourceThreadId', 'source-thread-2'],
    ['sourceSnapshotSha256', 'b'.repeat(64)],
  ]) {
    const mismatched = structuredClone(product);
    mismatched.binding[field] = replacement;
    assert.throws(
      () => validateProductSourceCheckpointBinding(mismatched, input),
      /source checkpoint binding does not match/,
    );
  }
});

test('fake provider executes paired strategies, preserves native handoff bytes, and resumes idempotently', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: expectedFinal() });
  const lock = campaignLock();
  const first = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: lock,
    scheduledArms: twoArms(),
    ...paths,
  });
  assert.equal(first.status, 'complete');
  assert.equal(first.completedNow, 2);
  const results = readJsonLines(paths.resultsPath);
  assert.equal(results.length, 2);
  const native = results.find((record) => record.payload.arm.strategy === 'native_handoff');
  assert.equal(native.payload.handoff.sha256, native.payload.handoff.deliveredSha256);
  assert.equal(native.payload.provenance.rawPromptRetained, false);
  assert.equal(native.payload.oracle.test.passed, false);
  assert.equal(native.payload.oracle.test.oracle.passed, false);
  assert.ok(native.payload.metrics.invariantViolationCount > 0);
  assert.deepEqual(
    native.payload.metrics.invariantViolations,
    native.payload.oracle.test.oracle.violatedInvariantIds,
  );
  assert.equal(native.payload.model.actualSnapshotId, 'gpt-5.5');
  assert.equal(results.every((record) => record.payload.metrics.success === false), true);
  const turns = provider.calls.filter((call) => call.method === 'turn/start');
  const sourceInitial = turns.find((call) => call.options.text.includes(`Scenario id: ${FIXTURE.fixture.id}`));
  assert.match(sourceInitial.options.text, new RegExp(FIXTURE.fixture.finalChallenge.completionMarker, 'u'));
  const finalChallenges = turns.filter((call) => call.options.text.includes('Complete the pending coding work from the preserved workflow state.'));
  assert.equal(finalChallenges.length, 2);
  assert.equal(finalChallenges.every((call) => !call.options.text.includes(FIXTURE.fixture.goal)), true);
  for (const record of results) assertMatchesSchema(record, RESULT_SCHEMA);
  for (const record of readJsonLines(paths.controlPath)) assertMatchesSchema(record, CONTROL_SCHEMA);

  const resumed = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: lock,
    scheduledArms: twoArms(),
    ...paths,
  });
  assert.equal(resumed.completedNow, 0);
  assert.equal(readJsonLines(paths.resultsPath).length, 2);
});

test('product arm reuses the corresponding native handoff bytes and resumes idempotently', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: expectedFinal() });
  const native = twoArms().find((arm) => arm.strategy === 'native_handoff');
  const product = productArm();
  const workspace = {
    prepareArmWorkspace: async ({ armKey }) => ({
      repositoryRoot: paths.root,
      workspaceId: sha256(armKey),
      headSha: 'b'.repeat(40),
      templateSha256: syntheticTemplateDigests(ALL_FIXTURES)[FIXTURE.fixture.id],
    }),
    inspectWorkspaceChanges: async () => ({
      changedFiles: [],
      unexpectedFiles: [],
      changeDigestSha256: 'c'.repeat(64),
    }),
    runFixtureTest: async () => ({
      argv: ['fake-fixture-test'],
      passed: true,
      declaredPassed: true,
      declaredTestCount: 1,
      requiredTestCount: 1,
      executionCountPassed: true,
      oracle: { passed: true, violatedInvariantIds: [] },
      exitCode: 0,
      signal: null,
      timedOut: false,
      durationMs: 1,
      stdout: '',
      stderr: '',
      stdoutBytes: 0,
      stderrBytes: 0,
      outputTruncated: false,
    }),
  };
  const lock = campaignLock();
  const nativeClock = [0, 10, 20];
  await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: lock,
    scheduledArms: [native],
    monotonicNow: () => nativeClock.shift(),
    workspace,
    ...paths,
  });
  const handoffTurnCount = provider.calls.filter((call) =>
    call.method === 'turn/start' && /Prepare a prompt-ready handoff/.test(call.options?.text ?? '')
  ).length;
  const clock = [100, 140, 200, 250];
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: lock,
    scheduledArms: [product],
    productContextFactory: async ({ source }) => productContextFor(source),
    monotonicNow: () => clock.shift(),
    workspace,
    ...paths,
  };
  const result = await runCampaign(input);

  assert.equal(result.completedNow, 1);
  assert.equal(clock.length, 0);
  const records = readJsonLines(paths.resultsPath);
  const nativeRecord = records.find((record) => record.payload.arm.strategy === 'native_handoff');
  const record = records.find((entry) => entry.payload.arm.strategy === 'verified_context_pack_contracts');
  assert.equal(provider.calls.filter((call) =>
    call.method === 'turn/start' && /Prepare a prompt-ready handoff/.test(call.options?.text ?? '')
  ).length, handoffTurnCount);
  assert.equal(record.payload.handoff.sha256, nativeRecord.payload.handoff.sha256);
  assert.equal(record.payload.handoff.deliveredSha256, nativeRecord.payload.handoff.sha256);
  assert.equal(record.payload.handoff.reusedFromNativeArmKey, native.key);
  assert.equal(record.payload.handoff.nativeResultRecordSha256, nativeRecord.recordSha256);
  assert.equal(record.payload.handoff.checkpointSha256, nativeRecord.payload.handoff.checkpointSha256);
  assertMatchesSchema(record, RESULT_SCHEMA);
  assert.equal(record.payload.productContext.generationMs, 30);
  assert.equal(record.payload.productContext.loadingMs, 40);
  assert.equal(record.payload.productContext.materializationMs, 70);
  assert.ok(Number.isFinite(record.payload.metrics.timing.completionMs));
  assert.ok(record.payload.metrics.timing.completionMs < record.payload.metrics.timing.endToEndMs);
  assert.equal(record.payload.metrics.timing.endToEndMs, 130);

  const turnCount = provider.calls.filter((call) => call.method === 'turn/start').length;
  const resumed = await runCampaign({ ...input, resume: true, monotonicNow: () => 300 });
  assert.equal(resumed.completedNow, 0);
  assert.equal(provider.calls.filter((call) => call.method === 'turn/start').length, turnCount);
});

test('product context fails closed when its source snapshot or checkpoint identity differs', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: expectedFinal() });
  const native = twoArms().find((arm) => arm.strategy === 'native_handoff');
  const product = productArm();
  const workspace = successfulWorkspace(paths);
  const lock = campaignLock();
  await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: lock,
    scheduledArms: [native],
    workspace,
    ...paths,
  });
  const handoffTurnCount = provider.calls.filter((call) =>
    call.method === 'turn/start' && /Prepare a prompt-ready handoff/.test(call.options?.text ?? '')
  ).length;
  const result = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: lock,
    scheduledArms: [product],
    productContextFactory: async ({ source }) => productContextFor(source, {
      sourceCompaction: source.compaction + 1,
      sourceSnapshotSha256: 'b'.repeat(64),
    }),
    workspace,
    ...paths,
  });

  assert.equal(result.completedNow, 0);
  const abandoned = readJsonLines(paths.controlPath).find((record) => record.event === 'attempt_abandoned');
  assert.match(abandoned.message, /source checkpoint binding does not match/);
  assert.equal(provider.calls.filter((call) =>
    call.method === 'turn/start' && /Prepare a prompt-ready handoff/.test(call.options?.text ?? '')
  ).length, handoffTurnCount);
});

test('product arm rejects a native terminal handoff SHA that differs from its paid-stage checkpoint', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: expectedFinal() });
  const native = twoArms().find((arm) => arm.strategy === 'native_handoff');
  const workspace = successfulWorkspace(paths);
  const lock = campaignLock();
  await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: lock,
    scheduledArms: [native],
    workspace,
    ...paths,
  });
  const [nativeRecord] = readJsonLines(paths.resultsPath);
  nativeRecord.payload.handoff.sha256 = 'd'.repeat(64);
  nativeRecord.payload.handoff.deliveredSha256 = 'd'.repeat(64);
  const { recordSha256: _oldHash, ...body } = nativeRecord;
  nativeRecord.recordSha256 = sha256(stableStringify(body));
  await writeFile(paths.resultsPath, `${JSON.stringify(nativeRecord)}\n`);
  const turnCount = provider.calls.filter((call) => call.method === 'turn/start').length;

  const result = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: lock,
    scheduledArms: [productArm()],
    productContextFactory: async ({ source }) => productContextFor(source),
    workspace,
    ...paths,
  });

  assert.equal(result.completedNow, 0);
  const abandoned = readJsonLines(paths.controlPath).findLast((record) => record.event === 'attempt_abandoned');
  assert.match(abandoned.message, /native handoff result and paid-stage checkpoint do not match/);
  assert.equal(provider.calls.filter((call) => call.method === 'turn/start').length, turnCount);
});

test('retryable provider failure is abandoned, then resumes only the missing arm', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: expectedFinal(), failTurnNumbers: [2] });
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: twoArms(),
    ...paths,
  };
  const first = await runCampaign(input);
  assert.equal(first.completedNow, 1);
  assert.equal(readJsonLines(paths.resultsPath).length, 1);
  assert.equal(readJsonLines(paths.controlPath).some((record) => record.event === 'attempt_abandoned'), true);

  const resumed = await runCampaign(input);
  assert.equal(resumed.completedNow, 1);
  assert.equal(readJsonLines(paths.resultsPath).length, 2);
  const attempts = readJsonLines(paths.controlPath).filter((record) => record.event === 'attempt_started');
  assert.equal(Math.max(...attempts.map((record) => record.attempt)), 2);
});

test('official rate-limit guard pauses at 80%, predicted crossing, and unavailable coverage', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ rateLimitSnapshots: [rateLimitSnapshot(80)], finalResponse: expectedFinal() });
  const result = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [twoArms()[0]],
    ...paths,
  });
  assert.equal(result.status, 'paused');
  assert.equal(result.pause.reason, 'rate_limit_threshold');
  assert.equal(provider.calls.some((call) => call.method === 'thread/start'), false);
  assert.equal(readJsonLines(paths.controlPath).at(-1).event, 'pause');

  assert.equal(shouldPauseForUsage(rateLimitSnapshot(79), 1, 80).reason, 'predicted_rate_limit_threshold');
  assert.equal(shouldPauseForUsage({}, 0, 80).reason, 'rate_limit_unavailable');
  assert.equal(shouldPauseForUsage(rateLimitSnapshot(79), 0, 80).paused, false);
});

test('an arm that crosses 80% is durably recorded before the campaign pauses', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({
    rateLimitSnapshots: [rateLimitSnapshot(70), rateLimitSnapshot(72), rateLimitSnapshot(74), rateLimitSnapshot(80)],
    finalResponse: expectedFinal(),
  });
  const result = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [twoArms()[0]],
    ...paths,
  });
  assert.equal(result.status, 'paused');
  assert.equal(result.completedNow, 1);
  assert.equal(readJsonLines(paths.resultsPath).length, 1);
  assert.equal(result.pause.usedPercent, 80);
});

test('native handoff pauses at the 80% guard before starting the fresh challenge', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({
    rateLimitSnapshots: [rateLimitSnapshot(0), rateLimitSnapshot(0), rateLimitSnapshot(79), rateLimitSnapshot(80)],
    finalResponse: expectedFinal(),
  });
  const native = twoArms().find((arm) => arm.strategy === 'native_handoff');
  const result = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [native],
    ...paths,
  });
  assert.equal(result.status, 'paused');
  assert.equal(result.completedNow, 0);
  const paidTurns = provider.calls.filter((call) => call.method === 'turn/start');
  assert.equal(paidTurns.length, 2);
  assert.match(paidTurns.at(-1).options.text, /Prepare a prompt-ready handoff/);
  assert.equal(readJsonLines(paths.controlPath).some((record) => record.event === 'attempt_paused'), true);
});

test('resume after the source-initial post guard never repeats the paid source turn', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({
    rateLimitSnapshots: [rateLimitSnapshot(0), new Error('temporary rate-limit read failure')],
    finalResponse: expectedFinal(),
  });
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [twoArms()[0]],
    ...paths,
  };
  const first = await runCampaign(input);
  assert.equal(first.status, 'paused');
  assert.equal(provider.calls.filter((call) => call.method === 'turn/start').length, 1);
  provider.rateLimitSnapshots.push(...Array.from({ length: 4 }, () => rateLimitSnapshot(0)));

  const resumed = await runCampaign({ ...input, resume: true });
  assert.equal(resumed.completedNow, 1);
  const paidTurns = provider.calls.filter((call) => call.method === 'turn/start');
  assert.equal(paidTurns.length, 2);
  assert.equal(paidTurns.filter((call) => call.options.text.includes(`Scenario id: ${FIXTURE.fixture.id}`)).length, 1);
});

test('resume after a worklog post guard compacts the retained turn without paying for it twice', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({
    rateLimitSnapshots: [
      rateLimitSnapshot(0),
      rateLimitSnapshot(0),
      rateLimitSnapshot(0),
      new Error('temporary rate-limit read failure'),
    ],
    finalResponse: expectedFinal(),
  });
  const arm = { ...twoArms()[0], compaction: 1 };
  arm.key = [arm.model, arm.scenario, arm.strategy, arm.compaction, arm.repetition, arm.fixtureSha256].join('/');
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [arm],
    ...paths,
  };
  const first = await runCampaign(input);
  assert.equal(first.status, 'paused');
  assert.equal(provider.calls.filter((call) => call.method === 'turn/start').length, 2);
  provider.rateLimitSnapshots.push(...Array.from({ length: 5 }, () => rateLimitSnapshot(0)));

  const resumed = await runCampaign({ ...input, resume: true });
  assert.equal(resumed.completedNow, 1);
  const paidTurns = provider.calls.filter((call) => call.method === 'turn/start');
  assert.equal(paidTurns.length, 3);
  assert.equal(paidTurns.filter((call) => call.options.text.startsWith('Worklog turn 1:')).length, 1);
  assert.equal(provider.calls.filter((call) => call.method === 'thread/compact/start').length, 1);
});

test('resume delivers the exact retained native handoff without regenerating it', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({
    rateLimitSnapshots: [
      rateLimitSnapshot(0),
      rateLimitSnapshot(0),
      rateLimitSnapshot(0),
      new Error('temporary rate-limit read failure'),
    ],
    finalResponse: expectedFinal(),
  });
  const native = twoArms().find((arm) => arm.strategy === 'native_handoff');
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [native],
    ...paths,
  };
  const first = await runCampaign(input);
  assert.equal(first.status, 'paused');
  const checkpoint = readJsonLines(paths.controlPath).find((record) =>
    record.event === 'paid_stage_checkpoint' && record.kind === 'native_handoff',
  );
  assert.match(checkpoint.handoffSha256, /^[0-9a-f]{64}$/u);
  provider.rateLimitSnapshots.push(...Array.from({ length: 4 }, () => rateLimitSnapshot(0)));

  const resumed = await runCampaign({ ...input, resume: true });
  assert.equal(resumed.completedNow, 1);
  const paidTurns = provider.calls.filter((call) => call.method === 'turn/start');
  assert.equal(paidTurns.filter((call) => /Prepare a prompt-ready handoff/.test(call.options.text)).length, 1);
  const [result] = readJsonLines(paths.resultsPath);
  assert.equal(result.payload.handoff.sha256, checkpoint.handoffSha256);
  assert.equal(result.payload.handoff.deliveredSha256, checkpoint.handoffSha256);
});

test('resume rejects a tampered paid-stage checkpoint before another paid turn', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({
    rateLimitSnapshots: [rateLimitSnapshot(0), new Error('temporary rate-limit read failure')],
    finalResponse: expectedFinal(),
  });
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [twoArms()[0]],
    ...paths,
  };
  await runCampaign(input);
  const records = readJsonLines(paths.controlPath);
  const checkpoint = records.find((record) => record.event === 'paid_stage_checkpoint');
  checkpoint.threadId = 'tampered-thread';
  await writeFile(paths.controlPath, `${records.map((record) => JSON.stringify(record)).join('\n')}\n`);
  const turnCount = provider.calls.filter((call) => call.method === 'turn/start').length;

  await assert.rejects(runCampaign({ ...input, resume: true }), /invalid content hash/);
  assert.equal(provider.calls.filter((call) => call.method === 'turn/start').length, turnCount);
});

test('resume rejects a tampered source checkpoint before another paid turn', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: expectedFinal(), failTurnNumbers: [2] });
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [twoArms()[0]],
    ...paths,
  };
  await runCampaign(input);
  const records = readJsonLines(paths.controlPath);
  const checkpoint = records.find((record) => record.event === 'source_checkpoint');
  checkpoint.snapshotThreadId = 'tampered-source-thread';
  await writeFile(paths.controlPath, `${records.map((record) => JSON.stringify(record)).join('\n')}\n`);
  const turnCount = provider.calls.filter((call) => call.method === 'turn/start').length;

  await assert.rejects(runCampaign({ ...input, resume: true }), /source_checkpoint .* invalid content hash/);
  assert.equal(provider.calls.filter((call) => call.method === 'turn/start').length, turnCount);
});

test('resume rejects a source checkpoint whose official thread snapshot changed', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: expectedFinal(), failTurnNumbers: [2] });
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [twoArms()[0]],
    ...paths,
  };
  await runCampaign(input);
  const checkpoint = readJsonLines(paths.controlPath).find((record) => record.event === 'source_checkpoint');
  provider.threads.get(checkpoint.snapshotThreadId).turns.push('out-of-ledger mutation');
  const turnCount = provider.calls.filter((call) => call.method === 'turn/start').length;

  await assert.rejects(
    runCampaign({ ...input, resume: true }),
    /thread\/read snapshot digest does not match/,
  );
  assert.equal(provider.calls.filter((call) => call.method === 'turn/start').length, turnCount);
});

test('resume recovers a crash after compaction without compacting the source twice', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({
    finalResponse: expectedFinal(),
    failAfterCompactNumbers: [1],
  });
  const arm = { ...twoArms()[0], compaction: 1 };
  arm.key = [arm.model, arm.scenario, arm.strategy, arm.compaction, arm.repetition, arm.fixtureSha256].join('/');
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [arm],
    ...paths,
  };

  await assert.rejects(runCampaign(input), /crash after compaction 1/);
  const beforeResume = readJsonLines(paths.controlPath);
  assert.equal(beforeResume.filter((record) => record.event === 'source_compaction_intent').length, 1);
  assert.equal(beforeResume.filter((record) => record.event === 'source_compaction_completed').length, 0);
  assert.equal(beforeResume.filter((record) => record.event === 'source_checkpoint').length, 1);

  const resumed = await runCampaign({ ...input, resume: true });
  assert.equal(resumed.completedNow, 1);
  assert.equal(provider.calls.filter((call) => call.method === 'thread/compact/start').length, 1);
  const afterResume = readJsonLines(paths.controlPath);
  const completion = afterResume.find((record) => record.event === 'source_compaction_completed');
  assert.equal(completion.recoveredAfterCrash, true);
  assert.match(completion.recordSha256, /^[0-9a-f]{64}$/u);
  assert.deepEqual(
    afterResume.filter((record) => record.event === 'source_checkpoint').map((record) => record.sourceSequence),
    [0, 1],
  );
});

test('resume restores the conservative usage increment before retrying a missing arm', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({
    rateLimitSnapshots: [
      rateLimitSnapshot(70),
      rateLimitSnapshot(72),
      rateLimitSnapshot(74),
      rateLimitSnapshot(79),
    ],
    finalResponse: expectedFinal(),
    failTurnNumbers: [2],
  });
  const input = {
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [twoArms()[0]],
    ...paths,
  };
  const first = await runCampaign(input);
  assert.equal(first.completedNow, 0);
  const lockSha = sha256(stableStringify(input.campaignLock));
  assert.equal(
    restoreUsageState(readJsonLines(paths.controlPath), { phase: 'measured', campaignLockSha256: lockSha })
      .maximumObservedUsageIncrement,
    2,
  );
  const turnCount = provider.calls.filter((call) => call.method === 'turn/start').length;
  const resumed = await runCampaign({ ...input, resume: true });
  assert.equal(resumed.status, 'paused');
  assert.equal(resumed.pause.reason, 'predicted_rate_limit_threshold');
  assert.equal(provider.calls.filter((call) => call.method === 'turn/start').length, turnCount);
});

test('invalid strict final JSON is a terminal measured model error', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: 'not-an-object' });
  const result = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [twoArms()[0]],
    ...paths,
  });
  assert.equal(result.completedNow, 1);
  const [record] = readJsonLines(paths.resultsPath);
  assert.equal(record.event, 'arm_model_error');
  assert.ok(record.payload.metrics.seriousErrors.includes('invalid_final_response'));
  assert.equal(readJsonLines(paths.controlPath).some((entry) => entry.event === 'attempt_abandoned'), false);
});

test('conflicting paid-stage model identities fail the measured arm closed', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: expectedFinal() });
  const runTurn = provider.runTurn.bind(provider);
  provider.runTurn = async (options) => {
    const result = await runTurn(options);
    if (options.text.includes('Return only one strict JSON object')) {
      result.modelVerification = [{ snapshotId: 'gpt-5.5-different-snapshot', verified: true }];
    }
    return result;
  };
  const result = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [twoArms()[0]],
    ...paths,
  });
  assert.equal(result.completedNow, 1);
  const [record] = readJsonLines(paths.resultsPath);
  assert.equal(record.event, 'arm_model_error');
  assert.ok(record.payload.metrics.seriousErrors.includes('model_identity_mismatch'));
  assert.equal(record.payload.model.allPaidStagesIdentified, true);
  assert.equal(record.payload.model.paidStageCount, 2);
  assert.equal(record.payload.model.actualSnapshotId.status, 'unavailable');
});

test('every intermediate source compaction is durably frozen for later odd refinement', async (t) => {
  const paths = await temporaryCase(t);
  const provider = new FakeProvider({ finalResponse: expectedFinal() });
  const armAtFour = { ...twoArms()[0], compaction: 4 };
  armAtFour.key = [armAtFour.model, armAtFour.scenario, armAtFour.strategy, armAtFour.compaction, armAtFour.repetition, armAtFour.fixtureSha256].join('/');
  await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [armAtFour],
    ...paths,
  });
  const sourceCheckpoints = readJsonLines(paths.controlPath)
    .filter((record) => record.event === 'source_checkpoint')
    .map((record) => record.compaction);
  assert.deepEqual(sourceCheckpoints, [0, 1, 2, 3, 4]);

  const armAtThree = { ...twoArms()[0], compaction: 3 };
  armAtThree.key = [armAtThree.model, armAtThree.scenario, armAtThree.strategy, armAtThree.compaction, armAtThree.repetition, armAtThree.fixtureSha256].join('/');
  const refined = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'measured',
    campaignLock: campaignLock(),
    scheduledArms: [armAtThree],
    resume: true,
    ...paths,
  });
  assert.equal(refined.completedNow, 1);
});

test('completed-arm resume validation rejects a corrupt append-only record hash', () => {
  const arm = twoArms()[0];
  const lockHash = sha256(stableStringify(campaignLock()));
  const record = {
    schemaVersion: 1,
    event: 'arm_completed',
    recordedAt: '2026-07-15T00:00:00.000Z',
    payload: {
      phase: 'measured',
      arm,
      binding: { campaignLockSha256: lockHash, fixtureSha256: arm.fixtureSha256 },
    },
    recordSha256: '0'.repeat(64),
  };
  assert.throws(() => completedArmKeys([record], { campaignLockSha256: lockHash }), /invalid record hash/);
});

test('measured execution accepts only hash-bound calibration results, control, and campaign lock', async (t) => {
  const paths = await temporaryCase(t);
  const models = MANIFEST.execution.models.map((model) => ({
    requested: model.id,
    catalogId: model.id,
    catalogModel: model.id,
    supportedReasoningEfforts: ['high'],
  }));
  const lock = buildCampaignLock({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    providerBinary: '/fake/codex',
    providerVersion: 'codex-cli fake',
    providerSha256: 'a'.repeat(64),
    models,
  });
  const lockPath = join(paths.root, 'campaign-lock.v1.json');
  writeJsonAtomic(lockPath, lock);
  const provider = new FakeProvider({ finalResponse: expectedFinal() });
  const result = await runCampaign({
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    provider,
    phase: 'calibration',
    campaignLock: lock,
    scheduledArms: twoArms(),
    ...paths,
  });
  assert.equal(result.completedNow, 2);
  const evidence = validateCalibrationEvidence({
    resultsPath: paths.resultsPath,
    controlPath: paths.controlPath,
    lockPath,
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    providerVersion: 'codex-cli fake',
    providerSha256: 'a'.repeat(64),
    models,
  });
  assert.equal(evidence.terminalArmCount, 2);
  assert.match(evidence.campaignLockSha256, /^[0-9a-f]{64}$/);
  assert.throws(() => validateCalibrationEvidence({
    resultsPath: paths.resultsPath,
    controlPath: paths.controlPath,
    lockPath,
    manifest: MANIFEST,
    fixtures: ALL_FIXTURES,
    providerVersion: 'codex-cli fake',
    providerSha256: 'b'.repeat(64),
    models,
  }), /does not match/);
});

function assertMatchesSchema(value, schema, root = schema, path = '$') {
  if (schema.$ref) {
    const target = schema.$ref.split('/').slice(1).reduce((current, key) => current[key], root);
    return assertMatchesSchema(value, target, root, path);
  }
  if (schema.const !== undefined) assert.deepEqual(value, schema.const, `${path} const`);
  if (schema.enum) assert.ok(schema.enum.includes(value), `${path} enum`);
  const types = Array.isArray(schema.type) ? schema.type : schema.type ? [schema.type] : [];
  if (types.length > 0) {
    const actual = value === null ? 'null' : Array.isArray(value) ? 'array' :
      Number.isInteger(value) ? 'integer' : typeof value;
    assert.ok(types.includes(actual) || (actual === 'integer' && types.includes('number')), `${path} type ${actual}`);
  }
  if (typeof value === 'string') {
    if (schema.minLength !== undefined) assert.ok(value.length >= schema.minLength, `${path} minLength`);
    if (schema.pattern) assert.match(value, new RegExp(schema.pattern, 'u'), `${path} pattern`);
  }
  if (typeof value === 'number' && schema.minimum !== undefined) assert.ok(value >= schema.minimum, `${path} minimum`);
  if (Array.isArray(value)) {
    if (schema.uniqueItems) assert.equal(new Set(value.map((item) => JSON.stringify(item))).size, value.length, `${path} uniqueItems`);
    if (schema.items) value.forEach((item, index) => assertMatchesSchema(item, schema.items, root, `${path}[${index}]`));
  } else if (value && typeof value === 'object') {
    for (const required of schema.required ?? []) assert.ok(Object.hasOwn(value, required), `${path}.${required} required`);
    if (schema.additionalProperties === false) {
      const allowed = new Set(Object.keys(schema.properties ?? {}));
      for (const key of Object.keys(value)) assert.ok(allowed.has(key), `${path}.${key} additional property`);
    }
    for (const [key, child] of Object.entries(schema.properties ?? {})) {
      if (Object.hasOwn(value, key)) assertMatchesSchema(value[key], child, root, `${path}.${key}`);
    }
  }
}
