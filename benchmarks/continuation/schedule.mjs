#!/usr/bin/env node

import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readJson, sha256, stableStringify, writeJsonAtomic } from './src/io.mjs';
import { buildContinuationSummary, parseAppendOnlyResults } from './src/summary.mjs';
import {
  buildAdditionalBaseMatrix,
  buildProductMatrix,
  loadFixtureSet,
  validateFixtureSet,
  validateManifest,
} from './src/validation.mjs';

const ROOT = dirname(fileURLToPath(import.meta.url));
const SCHEDULE_BOOTSTRAP = Object.freeze({
  iterations: 10_000,
  seed: 'previously-on-continuation-v1',
});

export function buildSchedule({ manifest, fixtures, summary, mode }) {
  const modelEntries = Object.entries(summary.models ?? {});
  let arms;
  let boundaries = [];
  if (mode === 'adaptive') {
    const schedules = modelEntries
      .map(([model, value]) => ({
        model,
        checkpoints: value.baseMatrix?.complete === true ? value.nextCheckpoints ?? [] : [],
      }))
      .filter((entry) => entry.checkpoints.length > 0);
    arms = schedules.length > 0 ? buildAdditionalBaseMatrix(manifest, fixtures, schedules) : [];
  } else if (mode === 'product') {
    boundaries = modelEntries.flatMap(([model, value]) => {
      if (
        value.baseMatrix?.complete !== true ||
        !value.degradationBoundary?.detected ||
        (value.nextCheckpoints ?? []).length > 0
      ) return [];
      return [{
        model,
        checkpoints: [
          value.degradationBoundary.boundaryCheckpoint,
          value.degradationBoundary.confirmedAtCheckpoint,
        ],
      }];
    });
    arms = boundaries.length > 0 ? buildProductMatrix(manifest, fixtures, boundaries) : [];
  } else {
    throw new Error('--mode adaptive|product is required');
  }
  return {
    schemaVersion: 1,
    benchmarkId: manifest.benchmarkId,
    mode,
    sourceSummarySha256: sha256(stableStringify(summary)),
    boundaries,
    refinementComplete: mode !== 'product' || boundaries.length > 0,
    armCount: arms.length,
    arms,
  };
}

export function buildEvidenceBoundSchedule({ manifest, fixtures, events, mode }) {
  const baseEvents = baseMeasuredEvents(events, manifest);
  if (baseEvents.length === 0) throw new Error('schedule requires current measured same-task/native-handoff results');
  const campaignLocks = [...new Set(baseEvents.map((event) => event.payload?.binding?.campaignLockSha256))];
  if (campaignLocks.length !== 1 || !/^[0-9a-f]{64}$/u.test(campaignLocks[0] ?? '')) {
    throw new Error('schedule evidence must contain exactly one valid measured campaign lock');
  }
  const summary = buildContinuationSummary({
    manifest,
    events: baseEvents,
    bootstrap: SCHEDULE_BOOTSTRAP,
  }).json;
  return {
    ...buildSchedule({ manifest, fixtures, summary, mode }),
    sourceCampaignLockSha256: campaignLocks[0],
    sourceResultsEvidenceSha256: sha256(stableStringify(baseEvents)),
  };
}

export function validateEvidenceBoundSchedule({ schedule, manifest, fixtures, events, campaignLockSha256 }) {
  const expected = buildEvidenceBoundSchedule({ manifest, fixtures, events, mode: schedule?.mode });
  if (expected.sourceCampaignLockSha256 !== campaignLockSha256) {
    throw new Error('schedule source evidence belongs to a different measured campaign lock');
  }
  if (stableStringify(schedule) !== stableStringify(expected)) {
    throw new Error('schedule does not exactly match the boundary and arm matrix recomputed from current base results');
  }
  return schedule;
}

function baseMeasuredEvents(events, manifest) {
  const strategies = new Set(manifest.matrix.initialStrategies);
  return events.filter((event) =>
    ['arm_completed', 'arm_model_error'].includes(event?.event) &&
    event.payload?.phase === 'measured' &&
    strategies.has(event.payload?.arm?.strategy),
  );
}

function parseArgs(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    const value = argv[++index];
    if (!key?.startsWith('--') || value === undefined) throw new Error(`invalid argument ${key ?? '<missing>'}`);
    values.set(key, value);
  }
  return values;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const manifest = validateManifest(readJson(resolve(args.get('--manifest') ?? join(ROOT, 'manifest.v1.json'))));
  const fixtures = validateFixtureSet(
    loadFixtureSet(resolve(args.get('--fixtures') ?? join(ROOT, 'fixtures'))),
    { manifest },
  );
  const summaryPath = args.get('--summary');
  const resultsPath = args.get('--results');
  const outputPath = args.get('--output');
  if (!summaryPath || !resultsPath || !outputPath) throw new Error('--summary, --results, and --output are required');
  const suppliedSummary = readJson(resolve(summaryPath));
  const events = parseAppendOnlyResults(readFileSync(resolve(resultsPath), 'utf8'));
  const schedule = buildEvidenceBoundSchedule({ manifest, fixtures, events, mode: args.get('--mode') });
  if (schedule.sourceSummarySha256 !== sha256(stableStringify(suppliedSummary))) {
    throw new Error('supplied summary does not match the current hash-bound base results');
  }
  writeJsonAtomic(resolve(outputPath), schedule);
  process.stdout.write(`${JSON.stringify({ mode: schedule.mode, armCount: schedule.armCount })}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`error: ${String(error?.message ?? error)}\n`);
    process.exitCode = 1;
  }
}
