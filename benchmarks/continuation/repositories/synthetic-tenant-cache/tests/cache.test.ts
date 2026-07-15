import assert from 'node:assert/strict';
import test from 'node:test';
import { TenantCache, TTL_MS } from '../src/cache.ts';

test('a cache hit invokes the loader only once', async () => {
  let calls = 0;
  const cache = new TenantCache();
  const loader = async () => ({ calls: ++calls });
  assert.deepEqual(await cache.get('alpha', 'record', loader), { calls: 1 });
  assert.deepEqual(await cache.get('alpha', 'record', loader), { calls: 1 });
  assert.equal(calls, 1);
});

test('an expired entry is loaded again after sixty seconds', async () => {
  let now = 0;
  let calls = 0;
  const cache = new TenantCache({ now: () => now });
  const loader = async () => ++calls;
  assert.equal(await cache.get('alpha', 'record', loader), 1);
  now = TTL_MS;
  assert.equal(await cache.get('alpha', 'record', loader), 2);
});
