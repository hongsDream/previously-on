import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { AppServerClient, maximumUsedPercent } from '../src/app-server-client.mjs';

const ROOT = dirname(fileURLToPath(import.meta.url));

test('official App Server client observes model identity, timing, tokens, tools, compaction, usage, and rate limits', async (t) => {
  const client = new AppServerClient({
    binary: process.execPath,
    args: [join(ROOT, 'fixtures/fake-app-server.mjs')],
    cwd: ROOT,
    timeoutMs: 5_000,
  });
  t.after(() => client.close());
  await client.start();
  assert.equal((await client.listModels())[0].model, 'gpt-5.5-snapshot-test');
  const limits = await client.readRateLimits();
  assert.equal(maximumUsedPercent(limits), 12);
  assert.equal((await client.readUsage()).summary.inputTokens, 10);

  const started = await client.startThread({ model: 'gpt-5.5', cwd: ROOT });
  assert.equal(started.resolvedModel, 'gpt-5.5-snapshot-test');
  const resumed = await client.resumeThread({ threadId: started.threadId, model: 'gpt-5.5', cwd: ROOT });
  assert.equal(resumed.threadId, started.threadId);
  const forked = await client.forkThread({ threadId: started.threadId, model: 'gpt-5.5', cwd: ROOT });
  assert.equal((await client.readThread(forked.threadId)).id, forked.threadId);
  const turn = await client.runTurn({
    threadId: forked.threadId,
    text: 'test',
    model: 'gpt-5.5',
    reasoningEffort: 'high',
    outputSchema: { type: 'object' },
    cwd: ROOT,
  });
  assert.equal(turn.finalText, '{"ok":true}');
  assert.equal(turn.turnStatus, 'completed');
  assert.equal(turn.tokenUsage.last.cachedInputTokens, 3);
  assert.equal(turn.toolCallCount, 1);
  assert.equal(turn.modelVerification[0].verified, true);
  assert.ok(turn.timing.ttftMs >= 0);
  assert.ok(turn.timing.completionMs >= turn.timing.ttftMs);
  const staleTurn = await client.runTurn({
    threadId: started.threadId,
    text: 'leave a completed turn buffered before compaction',
    model: 'gpt-5.5',
    reasoningEffort: 'high',
    cwd: ROOT,
  });
  const compacted = await client.compactThread(started.threadId);
  assert.equal(compacted.observed, true);
  assert.match(compacted.turnId, /^compact-/);
  assert.notEqual(compacted.turnId, staleTurn.turnId);
  assert.ok(compacted.durationMs >= 0);
  assert.ok(compacted.timing.completionMs >= compacted.timing.itemCompletionMs);
  assert.deepEqual(compacted.tokenUsage, {
    status: 'unavailable',
    reason: 'compaction_token_usage_notification_not_exposed',
  });
  assert.deepEqual(compacted.toolCalls, {
    status: 'unavailable',
    reason: 'compaction_tool_notifications_not_exposed',
  });
  assert.deepEqual(compacted.toolCallCount, {
    status: 'unavailable',
    reason: 'compaction_tool_notifications_not_exposed',
  });
  assert.deepEqual(compacted.modelVerification, {
    status: 'unavailable',
    reason: 'compaction_model_verification_not_exposed',
  });
  assert.equal(compacted.turnStatus, 'completed');
  assert.equal(compacted.turnError, null);
  const compactedAgain = await client.compactThread(started.threadId);
  assert.match(compactedAgain.turnId, /^compact-/);
  assert.notEqual(compactedAgain.turnId, compacted.turnId);
});

test('manual compaction waits for the same turn to complete after its context item completes', async (t) => {
  const client = new AppServerClient({
    binary: process.execPath,
    args: [join(ROOT, 'fixtures/fake-app-server.mjs')],
    cwd: ROOT,
    env: { ...process.env, FAKE_APP_SERVER_MANUAL_COMPACTION: '1' },
    timeoutMs: 5_000,
  });
  t.after(() => client.close());
  await client.start();
  const started = await client.startThread({ model: 'gpt-5.5', cwd: ROOT });

  let resolved = false;
  const compacting = client.compactThread(started.threadId).then((result) => {
    resolved = true;
    return result;
  });
  let status;
  do {
    status = await client.request('test/compaction/status');
  } while (!status.itemCompleted);

  assert.equal(status.turnCompleted, false);
  assert.equal(resolved, false);
  await client.request('test/compaction/complete', { turnId: status.turnId });
  const compacted = await compacting;
  assert.equal(compacted.turnId, status.turnId);
  assert.equal(compacted.turnStatus, 'completed');
});

test('manual compaction rejects a failed completion for the same turn', async (t) => {
  const client = new AppServerClient({
    binary: process.execPath,
    args: [join(ROOT, 'fixtures/fake-app-server.mjs')],
    cwd: ROOT,
    env: { ...process.env, FAKE_APP_SERVER_MANUAL_COMPACTION: '1' },
    timeoutMs: 5_000,
  });
  t.after(() => client.close());
  await client.start();
  const started = await client.startThread({ model: 'gpt-5.5', cwd: ROOT });

  const compacting = assert.rejects(
    client.compactThread(started.threadId),
    /ended with turn status failed/,
  );
  let status;
  do {
    status = await client.request('test/compaction/status');
  } while (!status.itemCompleted);
  await client.request('test/compaction/complete', {
    turnId: status.turnId,
    status: 'failed',
    error: { message: 'synthetic compaction failure' },
  });
  await compacting;
});

test('maximum rate-limit utilization accounts for primary, secondary, and per-limit windows', () => {
  assert.equal(maximumUsedPercent({
    rateLimits: { primary: { usedPercent: 5 }, secondary: { usedPercent: 50 } },
    rateLimitsByLimitId: { other: { primary: { usedPercent: 79 }, secondary: null } },
  }), 79);
  assert.equal(maximumUsedPercent({}), null);
});
