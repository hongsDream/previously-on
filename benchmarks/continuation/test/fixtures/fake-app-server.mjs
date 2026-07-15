#!/usr/bin/env node

import { createInterface } from 'node:readline';

let nextThread = 1;
let nextTurn = 1;
const pendingCompactions = new Map();
const manualCompactionCompletion = process.env.FAKE_APP_SERVER_MANUAL_COMPACTION === '1';

createInterface({ input: process.stdin, crlfDelay: Infinity }).on('line', (line) => {
  const request = JSON.parse(line);
  if (request.id === undefined) return;
  const respond = (result) => process.stdout.write(`${JSON.stringify({ id: request.id, result })}\n`);
  const notify = (method, params) => process.stdout.write(`${JSON.stringify({ method, params })}\n`);
  switch (request.method) {
    case 'initialize':
      respond({ userAgent: 'fake-app-server/1' });
      break;
    case 'model/list':
      respond({ data: [{ id: 'gpt-5.5', model: 'gpt-5.5-snapshot-test' }] });
      break;
    case 'account/rateLimits/read':
      respond({ rateLimits: { primary: { usedPercent: 12, resetsAt: 4102444800 }, secondary: null }, rateLimitsByLimitId: {} });
      break;
    case 'account/usage/read':
      respond({ summary: { inputTokens: 10, outputTokens: 2 } });
      break;
    case 'thread/start':
    case 'thread/fork':
    case 'thread/resume': {
      const threadId = request.method === 'thread/resume' ? request.params.threadId : `thread-${nextThread++}`;
      respond({ thread: { id: threadId }, model: 'gpt-5.5-snapshot-test', modelProvider: 'fake', reasoningEffort: 'high' });
      break;
    }
    case 'thread/read':
      respond({ thread: { id: request.params.threadId, turns: [], compacted: false } });
      break;
    case 'turn/start': {
      const turnId = `turn-${nextTurn++}`;
      const { threadId } = request.params;
      respond({ turn: { id: turnId } });
      notify('item/agentMessage/delta', { threadId, turnId, delta: '{' });
      notify('thread/tokenUsage/updated', {
        threadId,
        turnId,
        tokenUsage: {
          total: { inputTokens: 10, cachedInputTokens: 3, outputTokens: 2, reasoningOutputTokens: 1, totalTokens: 13 },
          last: { inputTokens: 10, cachedInputTokens: 3, outputTokens: 2, reasoningOutputTokens: 1, totalTokens: 13 },
          modelContextWindow: 200000,
        },
      });
      notify('model/verification', { threadId, turnId, verifications: [{ model: 'gpt-5.5-snapshot-test', verified: true }] });
      notify('item/completed', { threadId, turnId, item: { type: 'commandExecution', tool: 'exec_command', status: 'completed' } });
      notify('item/completed', { threadId, turnId, item: { type: 'agentMessage', text: '{"ok":true}' } });
      notify('turn/completed', { threadId, turn: { id: turnId, status: 'completed', error: null, items: [] } });
      break;
    }
    case 'thread/compact/start': {
      const turnId = `compact-${nextTurn++}`;
      const { threadId } = request.params;
      pendingCompactions.set(turnId, { threadId, itemCompleted: false, turnCompleted: false });
      respond({});
      setTimeout(() => {
        pendingCompactions.get(turnId).itemCompleted = true;
        notify('item/completed', {
          completedAtMs: Date.now(),
          threadId,
          turnId,
          item: { id: `compaction-item-${turnId}`, type: 'contextCompaction', status: 'completed' },
        });
      }, 10);
      setTimeout(() => notify('thread/compacted', { threadId, turnId }), 15);
      if (!manualCompactionCompletion) {
        setTimeout(() => completeCompaction(turnId, 'completed', null, notify), 20);
      }
      break;
    }
    case 'test/compaction/status': {
      const compaction = [...pendingCompactions.entries()].at(-1);
      respond(compaction
        ? { turnId: compaction[0], ...compaction[1] }
        : { turnId: null, itemCompleted: false, turnCompleted: false });
      break;
    }
    case 'test/compaction/complete': {
      const turnId = request.params?.turnId ?? [...pendingCompactions.keys()].at(-1);
      const status = request.params?.status ?? 'completed';
      const error = request.params?.error ?? null;
      completeCompaction(turnId, status, error, notify);
      respond({ turnId });
      break;
    }
    default:
      process.stdout.write(`${JSON.stringify({ id: request.id, error: { code: -32601, message: `unknown ${request.method}` } })}\n`);
  }
});

function completeCompaction(turnId, status, error, notify) {
  const compaction = pendingCompactions.get(turnId);
  if (!compaction) return;
  compaction.turnCompleted = true;
  notify('turn/completed', {
    threadId: compaction.threadId,
    turn: { id: turnId, status, error, items: [] },
  });
}
