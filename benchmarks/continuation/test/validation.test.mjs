import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  buildBaseMatrix,
  buildAdditionalBaseMatrix,
  buildProductMatrix,
  fixtureSetDigest,
  loadFixtureSet,
  validateFixture,
  validateFixtureSet,
  validateManifest,
} from '../src/validation.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const MANIFEST = JSON.parse(readFileSync(join(ROOT, 'manifest.v1.json'), 'utf8'));
const FIXTURES = loadFixtureSet(join(ROOT, 'fixtures'));
const CONTINUATION_RESULT_SCHEMA = JSON.parse(
  readFileSync(join(ROOT, 'schemas/continuation-result.v1.schema.json'), 'utf8'),
);

test('strict App Server output schema declares types for const and enum properties', () => {
  assert.deepEqual(CONTINUATION_RESULT_SCHEMA.properties.schemaVersion, { type: 'integer', const: 1 });
  assert.equal(CONTINUATION_RESULT_SCHEMA.properties.testStatus.type, 'string');
  assert.equal(CONTINUATION_RESULT_SCHEMA.additionalProperties, false);
  assert.equal(JSON.stringify(CONTINUATION_RESULT_SCHEMA).includes('uniqueItems'), false);
  assert.deepEqual(
    [...CONTINUATION_RESULT_SCHEMA.required].sort(),
    Object.keys(CONTINUATION_RESULT_SCHEMA.properties).sort(),
  );
});

test('versioned manifest and fixture set bind the verified SHA and exact 864-arm matrix', () => {
  validateManifest(MANIFEST);
  validateFixtureSet(FIXTURES, { manifest: MANIFEST });
  assert.equal(fixtureSetDigest(FIXTURES), '4f6ae416871a9a502377cad82396053883ab0aebaa479c016baad3d3f7363b4a');
  for (const { fixture } of FIXTURES) {
    assert.equal(fixture.worklogTurns.length, 20);
    const oracleBytes = readFileSync(join(ROOT, fixture.oracle.path));
    assert.equal(createHash('sha256').update(oracleBytes).digest('hex'), fixture.oracle.sha256);
  }
  const arms = buildBaseMatrix(MANIFEST, FIXTURES);
  assert.equal(arms.length, 864);
  assert.equal(new Set(arms.map((arm) => arm.key)).size, 864);
  assert.deepEqual([...new Set(arms.map((arm) => arm.strategy))], ['same_task', 'native_handoff']);
  assert.equal(arms.some((arm) => arm.strategy === 'verified_context_pack_contracts'), false);
});

test('product arms are conditional, model-specific, and outside the base count', () => {
  const arms = buildProductMatrix(MANIFEST, FIXTURES, [{ model: 'gpt-5.5', checkpoints: [7, 8] }]);
  assert.equal(arms.length, 48);
  assert.equal(arms.every((arm) => arm.model === 'gpt-5.5'), true);
  assert.equal(arms.every((arm) => arm.strategy === 'verified_context_pack_contracts'), true);
  assert.deepEqual([...new Set(arms.map((arm) => arm.compaction))], [7, 8]);
});

test('final challenge does not refill the continuation state being measured', () => {
  const prompts = new Set(FIXTURES.map(({ fixture }) => fixture.finalChallenge.prompt));
  assert.equal(prompts.size, 1);
  for (const { fixture } of FIXTURES) {
    const prompt = fixture.finalChallenge.prompt;
    assert.equal(prompt.includes(fixture.id), false, fixture.id);
    assert.equal(prompt.includes(fixture.goal), false, fixture.id);
    assert.equal(prompt.includes(fixture.finalChallenge.completionMarker), false, fixture.id);
    for (const file of fixture.changedFiles) assert.equal(prompt.includes(file.path), false, `${fixture.id}/${file.path}`);
    for (const invariant of fixture.invariants) assert.equal(prompt.includes(invariant.id), false, `${fixture.id}/${invariant.id}`);
    for (const stale of fixture.staleFacts) assert.equal(prompt.includes(stale.claim), false, `${fixture.id}/${stale.id}`);
  }
});

test('one model-specific additional checkpoint schedules 48 paired base arms', () => {
  const arms = buildAdditionalBaseMatrix(MANIFEST, FIXTURES, [{ model: 'gpt-5.6-sol', checkpoints: [17] }]);
  assert.equal(arms.length, 48);
  assert.equal(arms.every((arm) => arm.model === 'gpt-5.6-sol' && arm.compaction === 17), true);
});

test('validation fails closed on unknown fields, fixture drift, and shell-shaped argv', () => {
  const fixture = structuredClone(FIXTURES[0].fixture);
  fixture.unversioned = true;
  assert.throws(() => validateFixture(fixture), /unknown fields/);

  const drifted = structuredClone(MANIFEST);
  drifted.fixtureSet.expectedSha256 = '0'.repeat(64);
  assert.throws(() => validateFixtureSet(FIXTURES, { manifest: drifted }), /fixture-set SHA mismatch/);

  const shell = structuredClone(FIXTURES[0].fixture);
  shell.productArm.relevantContracts[0].requiredTests[0].shell = 'cargo test';
  assert.throws(() => validateFixture(shell), /stores a shell command/);

  const unpinnedOracle = structuredClone(FIXTURES[0].fixture);
  unpinnedOracle.oracle.sha256 = 'invalid';
  assert.throws(() => validateFixture(unpinnedOracle), /hidden oracle binding is invalid/);

  const misboundOracle = structuredClone(FIXTURES[0].fixture);
  misboundOracle.oracle.invariantIds.pop();
  assert.throws(() => validateFixture(misboundOracle), /hidden oracle binding is invalid/);

  const shortWorklog = structuredClone(FIXTURES[0].fixture);
  shortWorklog.worklogTurns.pop();
  assert.throws(() => validateFixture(shortWorklog), /twenty worklog turns/);
});
