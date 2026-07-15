import assert from 'node:assert/strict';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  buildEvidenceBoundSchedule,
  buildSchedule,
  validateEvidenceBoundSchedule,
} from '../schedule.mjs';
import { readJson, sha256, stableStringify } from '../src/io.mjs';
import { buildBaseMatrix, loadFixtureSet } from '../src/validation.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const manifest = readJson(join(ROOT, 'manifest.v1.json'));
const fixtures = loadFixtureSet(join(ROOT, 'fixtures'));

test('adaptive and product schedules remain separate from the 864 base matrix', () => {
  const adaptive = buildSchedule({
    manifest,
    fixtures,
    mode: 'adaptive',
    summary: {
      models: {
        'gpt-5.5': { baseMatrix: { complete: true }, nextCheckpoints: [18, 20] },
        'gpt-5.6-sol': { nextCheckpoints: [] },
      },
    },
  });
  assert.equal(adaptive.armCount, 96);
  assert.equal(adaptive.arms.every((arm) => arm.model === 'gpt-5.5'), true);

  const product = buildSchedule({
    manifest,
    fixtures,
    mode: 'product',
    summary: {
      models: {
        'gpt-5.5': {
          baseMatrix: { complete: true },
          nextCheckpoints: [],
          degradationBoundary: { detected: true, boundaryCheckpoint: 7, confirmedAtCheckpoint: 8 },
        },
      },
    },
  });
  assert.equal(product.armCount, 48);
  assert.equal(product.arms.every((arm) => arm.strategy === 'verified_context_pack_contracts'), true);
});

test('product scheduling fails closed until the complete base matrix is proved', () => {
  const product = buildSchedule({
    manifest,
    fixtures,
    mode: 'product',
    summary: {
      models: {
        'gpt-5.5': {
          baseMatrix: { complete: false },
          nextCheckpoints: [],
          degradationBoundary: { detected: true, boundaryCheckpoint: 7, confirmedAtCheckpoint: 8 },
        },
      },
    },
  });
  assert.equal(product.armCount, 0);
  assert.equal(product.refinementComplete, false);
});

test('adaptive scheduling fails closed until the complete base matrix is proved', () => {
  const adaptive = buildSchedule({
    manifest,
    fixtures,
    mode: 'adaptive',
    summary: {
      models: {
        'gpt-5.5': { baseMatrix: { complete: false }, nextCheckpoints: [18, 20] },
      },
    },
  });
  assert.equal(adaptive.armCount, 0);
});

test('execution schedules are exactly rebound to current terminal evidence and campaign lock', () => {
  const arm = buildBaseMatrix(manifest, fixtures)[0];
  const campaignLockSha256 = 'c'.repeat(64);
  const body = {
    schemaVersion: 1,
    event: 'arm_completed',
    recordedAt: '2026-07-15T00:00:00.000Z',
    payload: {
      phase: 'measured',
      arm,
      binding: {
        campaignLockSha256,
        fixtureSha256: arm.fixtureSha256,
        fixtureSetSha256: manifest.fixtureSet.expectedSha256,
        manifestSha256: sha256(stableStringify(manifest)),
        prerequisiteMergeSha: manifest.prerequisite.mergeSha,
      },
      model: {
        requested: arm.model,
        actualSnapshotId: `${arm.model}-snapshot`,
        allPaidStagesIdentified: true,
      },
      metrics: {
        success: true,
        seriousErrorCount: 0,
        stateRecall: { dimensions: {}, recalled: 4, total: 4, ratio: 1, allRecalled: true },
        timing: { completionMs: 100, endToEndMs: 120 },
      },
    },
  };
  const events = [{ ...body, recordSha256: sha256(stableStringify(body)) }];
  const schedule = buildEvidenceBoundSchedule({ manifest, fixtures, events, mode: 'adaptive' });
  assert.equal(schedule.sourceCampaignLockSha256, campaignLockSha256);
  assert.match(schedule.sourceResultsEvidenceSha256, /^[0-9a-f]{64}$/u);
  assert.equal(validateEvidenceBoundSchedule({
    schedule,
    manifest,
    fixtures,
    events,
    campaignLockSha256,
  }), schedule);

  const forged = structuredClone(schedule);
  forged.arms.push(arm);
  forged.armCount += 1;
  assert.throws(() => validateEvidenceBoundSchedule({
    schedule: forged,
    manifest,
    fixtures,
    events,
    campaignLockSha256,
  }), /does not exactly match/);
  assert.throws(() => validateEvidenceBoundSchedule({
    schedule,
    manifest,
    fixtures,
    events,
    campaignLockSha256: 'd'.repeat(64),
  }), /different measured campaign lock/);
});
