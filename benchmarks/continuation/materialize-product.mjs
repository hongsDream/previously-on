#!/usr/bin/env node

import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readJson, writeJsonAtomic } from './src/io.mjs';
import { materializeProductArm } from './src/product-arm.mjs';

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

async function main() {
  const args = parseArgs(process.argv.slice(2));
  for (const key of [
    '--previously-bin',
    '--data-dir',
    '--repository',
    '--repository-id',
    '--task-id',
    '--fixture-sha',
    '--fixture',
    '--base',
    '--head',
    '--source-key',
    '--source-compaction',
    '--source-thread-id',
    '--source-snapshot-sha',
    '--output',
  ]) {
    if (!args.get(key)) throw new Error(`${key} is required`);
  }
  const product = await materializeProductArm({
    previouslyBin: resolve(args.get('--previously-bin')),
    dataDir: resolve(args.get('--data-dir')),
    repository: resolve(args.get('--repository')),
    repositoryId: args.get('--repository-id'),
    taskId: args.get('--task-id'),
    fixtureSha256: args.get('--fixture-sha'),
    fixture: readJson(resolve(args.get('--fixture'))),
    base: args.get('--base'),
    head: args.get('--head'),
    sourceKey: args.get('--source-key'),
    sourceCompaction: Number(args.get('--source-compaction')),
    sourceThreadId: args.get('--source-thread-id'),
    sourceSnapshotSha256: args.get('--source-snapshot-sha'),
    tokenBudget: Number(args.get('--token-budget') ?? 1_200),
  });
  writeJsonAtomic(resolve(args.get('--output')), product);
  process.stdout.write(`${JSON.stringify({ sha256: product.sha256 })}\n`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`error: ${String(error?.message ?? error)}\n`);
    process.exitCode = 1;
  });
}
