#!/usr/bin/env node

import { existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readJson } from './src/io.mjs';
import { buildContinuationSummary, summarizeResultFiles } from './src/summary.mjs';

const ROOT = dirname(fileURLToPath(import.meta.url));

export function parseArgs(argv) {
  const result = new Map();
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    if (!key.startsWith('--')) throw new Error(`unexpected argument ${key}`);
    const value = argv[++index];
    if (value === undefined) throw new Error(`${key} requires a value`);
    result.set(key, value);
  }
  return result;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const manifest = readJson(resolve(args.get('--manifest') ?? join(ROOT, 'manifest.v1.json')));
  const resultsPath = resolve(args.get('--results') ?? join(ROOT, 'results/results.v1.jsonl'));
  const jsonPath = resolve(args.get('--json') ?? join(ROOT, 'results/threshold-recommendation.v1.json'));
  const markdownPath = resolve(args.get('--markdown') ?? join(ROOT, 'results/threshold-recommendation.v1.md'));
  const bootstrap = {
    iterations: Number(args.get('--bootstrap-iterations') ?? 10_000),
    seed: args.get('--bootstrap-seed') ?? 'previously-on-continuation-v1',
  };
  const output = existsSync(resultsPath)
    ? summarizeResultFiles({ resultsPath, manifest, jsonPath, markdownPath, bootstrap })
    : buildContinuationSummary({ manifest, events: [], bootstrap });
  if (!existsSync(resultsPath)) {
    mkdirSync(dirname(jsonPath), { recursive: true, mode: 0o700 });
    writeFileSync(jsonPath, `${JSON.stringify(output.json, null, 2)}\n`, { mode: 0o600 });
    writeFileSync(markdownPath, output.markdown, { mode: 0o600 });
  }
  process.stdout.write(`${JSON.stringify({ completedArms: output.json.completedArms, recommendation: output.json.recommendation.action })}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`error: ${String(error?.message ?? error)}\n`);
    process.exitCode = 1;
  }
}
