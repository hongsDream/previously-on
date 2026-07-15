import { readdirSync, readFileSync, statSync } from 'node:fs';
import { basename, join, sep } from 'node:path';
import { sha256, stableStringify } from './io.mjs';

const MODEL_IDS = ['gpt-5.5', 'gpt-5.6-sol'];
const BASE_STRATEGIES = ['same_task', 'native_handoff'];
const PRODUCT_STRATEGY = 'verified_context_pack_contracts';
const BASE_CHECKPOINTS = [0, 2, 4, 6, 8, 10, 12, 14, 16];
const MANIFEST_KEYS = [
  '$schema',
  'schemaVersion',
  'benchmarkId',
  'prerequisite',
  'execution',
  'matrix',
  'fairness',
  'metrics',
  'degradationBoundary',
  'productArmGate',
  'failureTaxonomy',
  'safety',
  'fixtureSet',
  'fixtures',
  'schemas',
];
const FIXTURE_KEYS = [
  'schemaVersion',
  'id',
  'family',
  'title',
  'repositorySnapshot',
  'goal',
  'invariants',
  'changedFiles',
  'staleFacts',
  'traps',
  'expectedState',
  'finalChallenge',
  'oracle',
  'rubric',
  'worklogTurns',
  'productArm',
];

export function loadFixtureSet(directory) {
  return listJsonFiles(directory)
    .map((path) => ({ path, fixture: JSON.parse(readFileSync(path, 'utf8')) }))
    .filter(({ fixture }) => fixture.schemaVersion === 1 && fixture.id)
    .sort((left, right) => left.fixture.id.localeCompare(right.fixture.id));
}

function listJsonFiles(directory) {
  const paths = [];
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) paths.push(...listJsonFiles(path));
    else if (entry.endsWith('.json')) paths.push(path);
  }
  return paths;
}

export function validateManifest(manifest) {
  assertObject(manifest, 'manifest');
  assertExactKeys(manifest, MANIFEST_KEYS, 'manifest');
  if (manifest.$schema !== 'schemas/manifest.v1.schema.json') throw new Error('manifest $schema changed');
  if (manifest.schemaVersion !== 1 || manifest.benchmarkId !== 'previously-on-continuation-v1') {
    throw new Error('manifest identity must be previously-on-continuation-v1 schemaVersion 1');
  }
  if (manifest.prerequisite?.taskId !== '019f63ad-a12b-78f2-8fa4-bb4c4a528078') {
    throw new Error('manifest prerequisite task changed');
  }
  if (
    manifest.prerequisite?.mergeSha !== '7aba2ba71a40b713be54d3540386fa4195026354' ||
    manifest.prerequisite?.requiredOnOriginMain !== true
  ) {
    throw new Error('manifest must bind the verified Regression Contracts origin/main merge SHA');
  }
  const models = manifest.execution?.models;
  if (!Array.isArray(models) || models.map((item) => item.id).join(',') !== MODEL_IDS.join(',')) {
    throw new Error(`manifest models must be ${MODEL_IDS.join(', ')}`);
  }
  for (const model of models) {
    assertExactKeys(model, ['id', 'reasoningEffort', 'fastMode', 'actualSnapshotRequired'], `model ${model.id}`);
    if (model.reasoningEffort !== 'high' || model.fastMode !== false || model.actualSnapshotRequired !== true) {
      throw new Error(`model ${model.id} must use high reasoning, fast mode off, and require observed identity`);
    }
  }
  if (manifest.execution?.calibrationExcludedFromMeasuredResults !== true) {
    throw new Error('calibration must be excluded from measured results');
  }
  const requiredMethods = new Set(manifest.execution?.officialAppServer?.requiredRequests ?? []);
  for (const method of [
    'initialize',
    'model/list',
    'thread/start',
    'thread/fork',
    'thread/compact/start',
    'turn/start',
    'account/rateLimits/read',
    'account/usage/read',
  ]) {
    if (!requiredMethods.has(method)) throw new Error(`official App Server requirements omitted ${method}`);
  }
  const matrix = manifest.matrix ?? {};
  if (JSON.stringify(matrix.initialStrategies) !== JSON.stringify(BASE_STRATEGIES)) throw new Error('base strategies changed');
  if (JSON.stringify(matrix.compactionCheckpoints) !== JSON.stringify(BASE_CHECKPOINTS)) throw new Error('base checkpoints changed');
  if (matrix.repetitions !== 3 || matrix.expectedBaseMeasuredArms !== 864 || matrix.scenarioCount !== 8) {
    throw new Error('manifest must declare eight scenarios, three repetitions, and 864 base measured arms');
  }
  if (matrix.conditionalProductStrategy?.id !== PRODUCT_STRATEGY || matrix.conditionalProductStrategy?.includedInBaseArmCount !== false) {
    throw new Error('conditional product strategy must be excluded from the base matrix');
  }
  if (JSON.stringify(matrix.extensionCheckpoints) !== JSON.stringify([18, 20])) {
    throw new Error('extension checkpoints must be 18 and 20');
  }
  if (manifest.safety?.rateLimitPauseAtUtilization !== 0.8) throw new Error('manifest must pause at 80% usage');
  for (const [key, expected] of [
    ['automaticCreditPurchase', false],
    ['automaticResetCreditConsumption', false],
    ['appendOnlyRawResults', true],
    ['resumeSkipsCompletedCheckpointKeys', true],
    ['secretRedactionRequired', true],
    ['releaseCompatibilityRerunForbidden', true],
    ['automaticTaskRolloverImplementation', false],
    ['cloudServicesAdded', false],
  ]) {
    if (manifest.safety?.[key] !== expected) throw new Error(`manifest safety.${key} must be ${expected}`);
  }
  if (manifest.degradationBoundary?.confidenceInterval?.method !== 'bootstrap' || manifest.degradationBoundary?.confidenceInterval?.level !== 0.95) {
    throw new Error('degradation boundary must use a 95% bootstrap confidence interval');
  }
  if (manifest.productArmGate?.failureRecommendation !== 'no_auto_rollover') {
    throw new Error('failed product gate must emit no_auto_rollover');
  }
  if (manifest.failureTaxonomy !== 'failure-taxonomy.v1.json') throw new Error('manifest failure taxonomy binding changed');
  if (!Array.isArray(manifest.fixtures) || manifest.fixtures.length !== 8) throw new Error('manifest must list eight fixtures');
  if (!/^[0-9a-f]{64}$/.test(manifest.fixtureSet?.expectedSha256 ?? '')) throw new Error('manifest fixture-set SHA is invalid');
  return manifest;
}

export function validateFixtureSet(entries, { manifest = null } = {}) {
  if (!Array.isArray(entries) || entries.length !== 8) throw new Error('fixture set must contain exactly eight scenarios');
  const ids = new Set();
  const families = new Map();
  for (const entry of entries) {
    const fixture = entry.fixture ?? entry;
    validateFixture(fixture);
    if (ids.has(fixture.id)) throw new Error(`duplicate fixture id ${fixture.id}`);
    ids.add(fixture.id);
    families.set(fixture.family, (families.get(fixture.family) ?? 0) + 1);
  }
  if (families.get('synthetic') !== 4 || families.get('previously_on') !== 4 || families.size !== 2) {
    throw new Error('fixture set must contain four synthetic and four previously_on scenarios');
  }
  if (manifest) {
    const listed = manifest.fixtures.map((entry) => `${entry.id}:${entry.family}:${entry.path}`).sort();
    const actual = entries
      .map(({ path, fixture }) => `${fixture.id}:${fixture.family}:fixtures/${basename(path)}`)
      .sort();
    if (JSON.stringify(listed) !== JSON.stringify(actual)) throw new Error('manifest fixture list does not match files on disk');
    const actualDigest = fixtureSetDigest(entries);
    if (manifest.fixtureSet.expectedSha256 !== actualDigest) {
      throw new Error(`fixture-set SHA mismatch: manifest=${manifest.fixtureSet.expectedSha256} actual=${actualDigest}`);
    }
  }
  return entries;
}

export function validateFixture(fixture) {
  assertObject(fixture, 'fixture');
  assertExactKeys(fixture, FIXTURE_KEYS, `fixture ${fixture.id ?? '<unknown>'}`);
  if (fixture.schemaVersion !== 1) throw new Error(`fixture ${fixture.id ?? '<unknown>'} schemaVersion must be 1`);
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(fixture.id ?? '')) throw new Error('fixture id must be a lowercase slug');
  if (!['synthetic', 'previously_on'].includes(fixture.family)) throw new Error(`fixture ${fixture.id} has invalid family`);
  if (!text(fixture.title) || !text(fixture.goal)) throw new Error(`fixture ${fixture.id} omitted title or goal`);
  if (!/^[0-9a-f]{40}$/.test(fixture.repositorySnapshot?.baseSha ?? '')) {
    throw new Error(`fixture ${fixture.id} repository snapshot omitted a 40-hex base SHA`);
  }
  if (!['synthetic_template', 'previously_on_merge'].includes(fixture.repositorySnapshot?.kind)) {
    throw new Error(`fixture ${fixture.id} repository snapshot kind is invalid`);
  }
  if (!nonemptyArray(fixture.invariants) || fixture.invariants.some((item) => !text(item.id) || !text(item.text))) {
    throw new Error(`fixture ${fixture.id} omitted named invariants`);
  }
  if (!nonemptyArray(fixture.changedFiles)) throw new Error(`fixture ${fixture.id} omitted changed files`);
  const changedPaths = fixture.changedFiles.flatMap((item) =>
    item.status === 'renamed' && typeof item.previousPath === 'string'
      ? [item.previousPath, item.path]
      : [item.path],
  );
  if (changedPaths.some((path) => !safeRelativePath(path)) || new Set(changedPaths).size !== changedPaths.length) {
    throw new Error(`fixture ${fixture.id} changed files are invalid or duplicated`);
  }
  if (!nonemptyArray(fixture.staleFacts) || fixture.staleFacts.some((item) => !text(item.id) || !text(item.claim) || !text(item.truth))) {
    throw new Error(`fixture ${fixture.id} omitted stale-fact truth oracles`);
  }
  if (!nonemptyArray(fixture.traps)) throw new Error(`fixture ${fixture.id} omitted stale challenge traps`);
  assertObject(fixture.expectedState, `fixture ${fixture.id} expectedState`);
  assertExactKeys(
    fixture.expectedState,
    ['goal', 'changedFiles', 'testStatus', 'nextStep', 'invariantViolations', 'staleClaims', 'seriousErrors'],
    `fixture ${fixture.id} expectedState`,
  );
  if (!text(fixture.expectedState.goal) || typeof fixture.expectedState.nextStep !== 'string') {
    throw new Error(`fixture ${fixture.id} expected state is incomplete`);
  }
  if (!['passed', 'failed', 'not_run', 'unavailable'].includes(fixture.expectedState.testStatus)) {
    throw new Error(`fixture ${fixture.id} expected testStatus is invalid`);
  }
  if (JSON.stringify([...fixture.expectedState.changedFiles].sort()) !== JSON.stringify([...changedPaths].sort())) {
    throw new Error(`fixture ${fixture.id} expected changed files diverge from the fixture change set`);
  }
  for (const key of ['invariantViolations', 'staleClaims', 'seriousErrors']) {
    if (!Array.isArray(fixture.expectedState[key]) || fixture.expectedState[key].length !== 0) {
      throw new Error(`fixture ${fixture.id} expected ${key} must be an empty array`);
    }
  }
  const challenge = fixture.finalChallenge;
  if (
    !text(challenge?.prompt) ||
    !text(challenge?.completionMarker) ||
    !nonemptyArray(challenge?.requiredTestCommand) ||
    challenge?.responseSchema !== 'schemas/continuation-result.v1.schema.json'
  ) {
    throw new Error(`fixture ${fixture.id} final challenge is incomplete`);
  }
  if (!challenge.requiredTestCommand.every(text)) throw new Error(`fixture ${fixture.id} required test argv is invalid`);
  if (JSON.stringify([...challenge.allowedFiles].sort()) !== JSON.stringify([...changedPaths].sort())) {
    throw new Error(`fixture ${fixture.id} allowed files diverge from the fixture change set`);
  }
  const oracle = fixture.oracle;
  const fixtureInvariantIds = fixture.invariants.map((item) => item.id).sort();
  const boundInvariantIds = Array.isArray(oracle?.invariantIds) ? [...oracle.invariantIds].sort() : [];
  if (
    oracle?.version !== 1
    || oracle.path !== `oracles/${fixture.id}.v1.mjs`
    || !/^[0-9a-f]{64}$/.test(oracle.sha256 ?? '')
    || !Number.isInteger(oracle.requiredCommandTestCount)
    || oracle.requiredCommandTestCount < 1
    || !Number.isInteger(oracle.minimumAssertionCount)
    || oracle.minimumAssertionCount < 1
    || stableStringify(boundInvariantIds) !== stableStringify(fixtureInvariantIds)
  ) {
    throw new Error(`fixture ${fixture.id} hidden oracle binding is invalid`);
  }
  const worklog = fixture.worklogTurns;
  if (!Array.isArray(worklog) || worklog.length !== 20) throw new Error(`fixture ${fixture.id} must contain twenty worklog turns`);
  worklog.forEach((item, index) => {
    if (item.turn !== index + 1 || !text(item.summary) || !text(item.stateDelta)) {
      throw new Error(`fixture ${fixture.id} worklog turn ${index + 1} is invalid`);
    }
  });
  const rubric = fixture.rubric;
  if (rubric?.maxScore !== 100 || !Number.isInteger(rubric.successThreshold) || !nonemptyArray(rubric.criteria)) {
    throw new Error(`fixture ${fixture.id} rubric is invalid`);
  }
  const points = rubric.criteria.reduce((sum, item) => sum + item.points, 0);
  if (points !== rubric.maxScore) throw new Error(`fixture ${fixture.id} rubric points must sum to 100`);
  const relevant = fixture.productArm?.relevantContracts;
  const irrelevant = fixture.productArm?.irrelevantContracts;
  if (!nonemptyArray(relevant) || !nonemptyArray(irrelevant)) {
    throw new Error(`fixture ${fixture.id} needs relevant and irrelevant Regression Contracts`);
  }
  if (fixture.productArm.strategyId !== PRODUCT_STRATEGY || fixture.productArm.activation !== 'after_boundary_only') {
    throw new Error(`fixture ${fixture.id} product arm activation changed`);
  }
  if (fixture.productArm.fixtureBaseSha !== fixture.repositorySnapshot.baseSha) {
    throw new Error(`fixture ${fixture.id} product-arm base SHA does not match repository snapshot`);
  }
  for (const contract of [...relevant, ...irrelevant]) validateRegressionContract(contract, fixture.id);
  if (!relevant.every((contract) => contract.status === 'active')) {
    throw new Error(`fixture ${fixture.id} relevant contracts must be active`);
  }
  const relevantIds = new Set(relevant.map((contract) => contract.id));
  if (irrelevant.some((contract) => relevantIds.has(contract.id))) {
    throw new Error(`fixture ${fixture.id} relevant and irrelevant contracts overlap`);
  }
  return fixture;
}

function validateRegressionContract(contract, fixtureId) {
  if (contract.schemaVersion !== 1 || !/^[0-9a-f-]{36}$/i.test(contract.id ?? '') || !text(contract.title) || !text(contract.invariant)) {
    throw new Error(`fixture ${fixtureId} has invalid Regression Contract identity`);
  }
  if (!['active', 'superseded'].includes(contract.status) || !nonemptyArray(contract.impactSelectors) || !nonemptyArray(contract.requiredTests)) {
    throw new Error(`fixture ${fixtureId} contract ${contract.id} omitted status, selectors, or tests`);
  }
  for (const test of contract.requiredTests) {
    if (!text(test.program) || !Array.isArray(test.args) || !text(test.workingDirectory) || !Number.isInteger(test.timeoutSeconds)) {
      throw new Error(`fixture ${fixtureId} contract ${contract.id} has invalid argv test`);
    }
    if ('command' in test || 'shell' in test) throw new Error(`fixture ${fixtureId} contract ${contract.id} stores a shell command`);
  }
}

export function fixtureDigest(fixture) {
  return sha256(stableStringify(fixture));
}

export function fixtureSetDigest(entries) {
  const ordered = entries
    .map((entry) => {
      const fixture = entry.fixture ?? entry;
      const path = entry.path ? `fixtures/${basename(entry.path)}` : `fixtures/${fixture.id}.json`;
      const bytes = entry.path ? readFileSync(entry.path) : Buffer.from(`${stableStringify(fixture)}\n`);
      return { path: path.split(sep).join('/'), bytes };
    })
    .sort((left, right) => left.path.localeCompare(right.path));
  const chunks = [];
  for (const item of ordered) chunks.push(Buffer.from(item.path), Buffer.from([0]), item.bytes, Buffer.from([0]));
  return sha256(Buffer.concat(chunks));
}

export function buildBaseMatrix(manifest, entries) {
  validateManifest(manifest);
  validateFixtureSet(entries, { manifest });
  const arms = buildMatrix({
    models: manifest.execution.models.map((model) => model.id),
    entries,
    strategies: BASE_STRATEGIES,
    checkpoints: manifest.matrix.compactionCheckpoints,
    repetitions: manifest.matrix.repetitions,
  });
  if (arms.length !== manifest.matrix.expectedBaseMeasuredArms || new Set(arms.map((arm) => arm.key)).size !== arms.length) {
    throw new Error(`base matrix expected ${manifest.matrix.expectedBaseMeasuredArms} unique arms but produced ${arms.length}`);
  }
  return arms;
}

export function buildProductMatrix(manifest, entries, boundaries) {
  validateManifest(manifest);
  validateFixtureSet(entries, { manifest });
  if (!Array.isArray(boundaries) || boundaries.length === 0) throw new Error('product matrix requires model-specific supported boundaries');
  const allowedModels = new Set(MODEL_IDS);
  const arms = [];
  for (const boundary of boundaries) {
    if (!allowedModels.has(boundary.model)) throw new Error(`product boundary model ${boundary.model} is not in the manifest`);
    if (!Array.isArray(boundary.checkpoints) || boundary.checkpoints.length !== 2) {
      throw new Error(`product boundary for ${boundary.model} must contain the two supported checkpoints`);
    }
    arms.push(...buildMatrix({
      models: [boundary.model],
      entries,
      strategies: [PRODUCT_STRATEGY],
      checkpoints: boundary.checkpoints,
      repetitions: manifest.matrix.repetitions,
    }));
  }
  if (new Set(arms.map((arm) => arm.key)).size !== arms.length) throw new Error('product matrix produced duplicate checkpoint keys');
  return arms;
}

export function buildAdditionalBaseMatrix(manifest, entries, schedules) {
  validateManifest(manifest);
  validateFixtureSet(entries, { manifest });
  if (!Array.isArray(schedules) || schedules.length === 0) throw new Error('additional base matrix requires model schedules');
  const arms = [];
  for (const schedule of schedules) {
    if (!MODEL_IDS.includes(schedule.model)) throw new Error(`additional schedule model ${schedule.model} is not in the manifest`);
    if (!Array.isArray(schedule.checkpoints) || schedule.checkpoints.length === 0) {
      throw new Error(`additional schedule for ${schedule.model} omitted checkpoints`);
    }
    arms.push(...buildMatrix({
      models: [schedule.model],
      entries,
      strategies: BASE_STRATEGIES,
      checkpoints: schedule.checkpoints,
      repetitions: manifest.matrix.repetitions,
    }));
  }
  if (new Set(arms.map((arm) => arm.key)).size !== arms.length) throw new Error('additional base matrix produced duplicate checkpoint keys');
  return arms;
}

function buildMatrix({ models, entries, strategies, checkpoints, repetitions }) {
  const arms = [];
  for (const model of models) {
    for (const { fixture } of entries) {
      const fixtureSha256 = fixtureDigest(fixture);
      for (const strategy of strategies) {
        for (const compaction of checkpoints) {
          for (let repetition = 1; repetition <= repetitions; repetition += 1) {
            const identity = { model, scenario: fixture.id, strategy, compaction, repetition, fixtureSha256 };
            arms.push({ ...identity, key: armKey(identity) });
          }
        }
      }
    }
  }
  return arms;
}

export function armKey({ model, scenario, strategy, compaction, repetition, fixtureSha256 }) {
  return `${model}/${scenario}/${strategy}/${compaction}/${repetition}/${fixtureSha256}`;
}

function assertObject(value, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label} must be an object`);
}

function assertExactKeys(value, allowed, label) {
  const extras = Object.keys(value).filter((key) => !allowed.includes(key));
  if (extras.length) throw new Error(`${label} contains unknown fields: ${extras.join(', ')}`);
}

function nonemptyArray(value) {
  return Array.isArray(value) && value.length > 0;
}

function text(value) {
  return typeof value === 'string' && value.trim().length > 0;
}

function safeRelativePath(value) {
  return text(value) && !value.startsWith('/') && !value.split(/[\\/]/u).includes('..');
}

export const BASE_MATRIX = Object.freeze({
  models: MODEL_IDS,
  strategies: BASE_STRATEGIES,
  checkpoints: BASE_CHECKPOINTS,
  productStrategy: PRODUCT_STRATEGY,
});
