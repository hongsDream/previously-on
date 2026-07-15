import { unavailable } from './io.mjs';

export class FakeProvider {
  constructor({
    rateLimitSnapshots = [rateLimitSnapshot(0)],
    finalResponse = defaultFinalResponse(),
    failTurnNumbers = [],
    failAfterCompactNumbers = [],
    modelCatalog = ['gpt-5.5', 'gpt-5.6-sol'],
  } = {}) {
    this.rateLimitSnapshots = structuredClone(rateLimitSnapshots);
    this.finalResponse = finalResponse;
    this.failTurnNumbers = new Set(failTurnNumbers);
    this.failAfterCompactNumbers = new Set(failAfterCompactNumbers);
    this.modelCatalog = modelCatalog.map((id) => ({ id, model: id, supportedReasoningEfforts: ['high'] }));
    this.calls = [];
    this.threads = new Map();
    this.nextThread = 1;
    this.nextTurn = 1;
    this.nextRateLimit = 0;
  }

  async start() {
    this.calls.push({ method: 'initialize' });
    return { userAgent: 'previously-on-fake-provider/1' };
  }

  async listModels() {
    this.calls.push({ method: 'model/list' });
    return structuredClone(this.modelCatalog);
  }

  async readRateLimits() {
    this.calls.push({ method: 'account/rateLimits/read' });
    if (this.rateLimitSnapshots.length === 0) throw new Error('fake rate-limit snapshot unavailable');
    const index = Math.min(this.nextRateLimit, this.rateLimitSnapshots.length - 1);
    this.nextRateLimit += 1;
    const value = this.rateLimitSnapshots[index];
    if (value instanceof Error) throw value;
    return structuredClone(value);
  }

  async readUsage() {
    this.calls.push({ method: 'account/usage/read' });
    return unavailable('fake_provider_has_no_billing_usage');
  }

  async startThread(options) {
    const threadId = `fake-thread-${this.nextThread++}`;
    this.threads.set(threadId, { ...structuredClone(options), turns: [], compactions: 0 });
    this.calls.push({ method: 'thread/start', threadId, options: structuredClone(options) });
    return threadResult(threadId, options.model);
  }

  async forkThread(options) {
    const source = this.threads.get(options.threadId);
    if (!source) throw new Error(`fake source thread ${options.threadId} does not exist`);
    const threadId = `fake-thread-${this.nextThread++}`;
    this.threads.set(threadId, {
      ...structuredClone(source),
      ...structuredClone(options),
      parentThreadId: options.threadId,
      turns: [...source.turns],
    });
    this.calls.push({ method: 'thread/fork', threadId, options: structuredClone(options) });
    return threadResult(threadId, options.model ?? source.model);
  }

  async resumeThread(options) {
    const thread = this.threads.get(options.threadId);
    if (!thread) throw new Error(`fake thread ${options.threadId} does not exist`);
    const { threadId: _threadId, ...resumedOptions } = structuredClone(options);
    Object.assign(thread, resumedOptions);
    this.calls.push({ method: 'thread/resume', threadId: options.threadId, options: structuredClone(options) });
    return threadResult(options.threadId, options.model ?? thread.model);
  }

  async readThread(threadId) {
    const thread = this.threads.get(threadId);
    if (!thread) throw new Error(`fake thread ${threadId} does not exist`);
    this.calls.push({ method: 'thread/read', threadId });
    return { id: threadId, ...structuredClone(thread) };
  }

  async runTurn(options) {
    const thread = this.threads.get(options.threadId);
    if (!thread) throw new Error(`fake thread ${options.threadId} does not exist`);
    const turnNumber = this.nextTurn++;
    this.calls.push({ method: 'turn/start', turnNumber, options: structuredClone(options) });
    if (this.failTurnNumbers.has(turnNumber)) throw new Error(`fake provider transport failure on turn ${turnNumber}`);
    thread.turns.push(options.text);
    const handoff = options.text.includes('Prepare a prompt-ready handoff for a fresh Codex task.');
    const finalChallenge = options.text.includes('Return only one strict JSON object');
    const finalText = handoff
      ? 'Goal and invariants preserved. Verified tests are pending. Reject stale facts. Execute the fixture challenge next.'
      : finalChallenge
        ? typeof this.finalResponse === 'function'
          ? JSON.stringify(this.finalResponse({ options, thread: structuredClone(thread), turnNumber }))
          : JSON.stringify(this.finalResponse)
        : 'Acknowledged current state without editing files.';
    return {
      threadId: options.threadId,
      turnId: `fake-turn-${turnNumber}`,
      finalText,
      timing: { ttftMs: 5 + turnNumber, completionMs: 20 + turnNumber },
      tokenUsage: {
        total: { inputTokens: 100, cachedInputTokens: 25, outputTokens: 20, reasoningOutputTokens: 10, totalTokens: 130 },
        last: { inputTokens: 100, cachedInputTokens: 25, outputTokens: 20, reasoningOutputTokens: 10, totalTokens: 130 },
        modelContextWindow: 200_000,
      },
      toolCalls: [],
      toolCallCount: 0,
      reroutes: [],
      modelVerification: [{ model: options.model, verified: true }],
      turnStatus: 'completed',
      turnError: null,
    };
  }

  async compactThread(threadId) {
    const thread = this.threads.get(threadId);
    if (!thread) throw new Error(`fake thread ${threadId} does not exist`);
    thread.compactions += 1;
    this.calls.push({ method: 'thread/compact/start', threadId });
    if (this.failAfterCompactNumbers.has(thread.compactions)) {
      this.failAfterCompactNumbers.delete(thread.compactions);
      throw new Error(`fake provider crash after compaction ${thread.compactions}`);
    }
    return { turnId: `fake-compact-${thread.compactions}`, durationMs: 7, observed: true };
  }

  async close() {
    this.calls.push({ method: 'close' });
  }
}

export function rateLimitSnapshot(usedPercent, { secondaryUsedPercent = null } = {}) {
  return {
    rateLimits: {
      primary: { usedPercent, resetsAt: 4_102_444_800 },
      secondary: secondaryUsedPercent === null
        ? null
        : { usedPercent: secondaryUsedPercent, resetsAt: 4_102_444_800 },
    },
    rateLimitsByLimitId: {},
  };
}

function threadResult(threadId, model) {
  return {
    threadId,
    requestedModel: model,
    resolvedModel: model,
    modelProvider: 'fake',
    reasoningEffort: 'high',
    serviceTier: null,
  };
}

function defaultFinalResponse() {
  return {
    assistantFinal: 'FAKE_COMPLETION',
    repository: { changedFiles: [], invariantViolations: [] },
    toolTrace: { commands: [] },
    stateRecall: { goal: '', changedFiles: [], testStatus: '', nextStep: '' },
    staleClaims: [],
    seriousErrors: [],
  };
}
