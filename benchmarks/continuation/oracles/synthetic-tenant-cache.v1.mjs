import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const fixtureId = 'synthetic-tenant-cache';
const repositoryRoot = resolve(process.argv[2]);
const violations = new Set();
let assertions = 0;
let module = null;
try {
  module = await import(`${pathToFileURL(resolve(repositoryRoot, 'src/cache.ts')).href}?oracle=${Date.now()}`);
} catch {}
const check = async (id, assertion) => {
  assertions += 1;
  try { await assertion(); } catch { violations.add(id); }
};

await check('ttl', () => assert.equal(module?.TTL_MS, 60_000));
let now = 0;
let calls = 0;
const cache = module ? new module.TenantCache({ now: () => now }) : null;
const loader = async ({ tenantId, resourceId }) => `${tenantId}/${resourceId}/${++calls}`;
await check('miss', async () => assert.equal(await cache?.get('alpha', 'record', loader), 'alpha/record/1'));
await check('miss', () => assert.equal(calls, 1));
await check('tenant-isolation', async () => assert.equal(await cache?.get('beta', 'record', loader), 'beta/record/2'));
await check('tenant-isolation', () => assert.equal(calls, 2));
await check('miss', async () => assert.equal(await cache?.get('alpha', 'record', loader), 'alpha/record/1'));
now = 59_999;
await check('ttl', async () => assert.equal(await cache?.get('alpha', 'record', loader), 'alpha/record/1'));
now = 60_000;
await check('ttl', async () => assert.equal(await cache?.get('alpha', 'record', loader), 'alpha/record/3'));
await check('miss', () => assert.equal(calls, 3));

const violatedInvariantIds = [...violations].sort();
process.stdout.write(`PREVIOUSLY_ON_HIDDEN_ORACLE_V1 ${JSON.stringify({ fixtureId, version: 1, assertions, violatedInvariantIds })}\n`);
if (violatedInvariantIds.length > 0) process.exitCode = 1;
