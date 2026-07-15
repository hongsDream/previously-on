import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const fixtureId = 'synthetic-config-guard';
const repositoryRoot = resolve(process.argv[2]);
const violations = new Set();
let assertions = 0;
let module = null;
try {
  module = await import(`${pathToFileURL(resolve(repositoryRoot, 'src/config.ts')).href}?oracle=${Date.now()}`);
} catch {}

const check = (invariantId, assertion) => {
  assertions += 1;
  try { assertion(); } catch { violations.add(invariantId); }
};
const parses = (input, field, expected) => check(
  field === 'safeMode' ? 'safe-default' : 'bounded-retries',
  () => assert.deepEqual(module?.parseConfig(input)?.[field], expected),
);
const rejects = (invariantId, input) => check(invariantId, () => {
  assert.ok(module?.ConfigError);
  assert.throws(() => module.parseConfig(input), module.ConfigError);
});

parses({}, 'safeMode', true);
check('unknown-keys', () => assert.throws(() => module.parseConfig({ surprise: true }), module.ConfigError));
parses({}, 'maxRetries', 2);
parses({ maxRetries: 0 }, 'maxRetries', 0);
parses({ maxRetries: 5 }, 'maxRetries', 5);
rejects('bounded-retries', { maxRetries: -1 });
rejects('bounded-retries', { maxRetries: 6 });
rejects('bounded-retries', { maxRetries: 1.5 });
rejects('bounded-retries', { maxRetries: '2' });

const violatedInvariantIds = [...violations].sort();
process.stdout.write(`PREVIOUSLY_ON_HIDDEN_ORACLE_V1 ${JSON.stringify({ fixtureId, version: 1, assertions, violatedInvariantIds })}\n`);
if (violatedInvariantIds.length > 0) process.exitCode = 1;
