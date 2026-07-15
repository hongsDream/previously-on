import assert from 'node:assert/strict';
import test from 'node:test';
import { ConfigError, parseConfig } from '../src/config.ts';

test('safeMode defaults to true', () => {
  assert.equal(parseConfig({}).safeMode, true);
});

test('unknown keys fail closed', () => {
  assert.throws(() => parseConfig({ surprise: true }), ConfigError);
});
