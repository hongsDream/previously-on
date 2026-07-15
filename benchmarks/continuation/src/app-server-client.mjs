import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { performance } from 'node:perf_hooks';
import { unavailable } from './io.mjs';

const MAX_LINE_BYTES = 8 * 1024 * 1024;
const MAX_STDERR_BYTES = 512 * 1024;

export class AppServerClient {
  constructor({ binary, args = [], cwd, env = process.env, timeoutMs = 15 * 60 * 1000 }) {
    this.binary = binary;
    this.args = args;
    this.cwd = cwd;
    this.env = env;
    this.timeoutMs = timeoutMs;
    this.child = null;
    this.nextId = 1;
    this.pending = new Map();
    this.notifications = [];
    this.waiters = new Set();
    this.completedCompactionTurnIds = new Set();
    this.stderr = '';
    this.initializeResult = null;
  }

  async start() {
    if (this.child) return this.initializeResult;
    this.child = spawn(this.binary, [...this.args, 'app-server'], {
      cwd: this.cwd,
      env: this.env,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    this.child.stderr.on('data', (chunk) => {
      this.stderr = `${this.stderr}${chunk.toString('utf8')}`.slice(-MAX_STDERR_BYTES);
    });
    this.child.once('exit', (code, signal) => {
      const error = new Error(`Codex app-server exited (${code ?? signal ?? 'unknown'})`);
      for (const { reject } of this.pending.values()) reject(error);
      this.pending.clear();
      for (const waiter of this.waiters) waiter.reject(error);
      this.waiters.clear();
    });
    const lines = createInterface({ input: this.child.stdout, crlfDelay: Infinity });
    lines.on('line', (line) => this.#receive(line));
    this.initializeResult = await this.request('initialize', {
      clientInfo: {
        name: 'previously_on_continuation_benchmark',
        title: 'PreviouslyOn Continuation Benchmark',
        version: '1.0.0',
      },
    });
    this.notify('initialized', {});
    return this.initializeResult;
  }

  #receive(line) {
    if (Buffer.byteLength(line) > MAX_LINE_BYTES) {
      this.#failAll(new Error(`app-server frame exceeded ${MAX_LINE_BYTES} bytes`));
      return;
    }
    let message;
    try {
      message = JSON.parse(line);
    } catch (error) {
      this.#failAll(new Error(`invalid JSON from app-server: ${error.message}`));
      return;
    }
    if (message.id !== undefined && this.pending.has(message.id)) {
      const pending = this.pending.get(message.id);
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (message.error) pending.reject(new RpcError(pending.method, message.error));
      else pending.resolve(message.result);
      return;
    }
    if (typeof message.method === 'string') {
      const observed = { ...message, observedAtMs: performance.now() };
      this.notifications.push(observed);
      if (this.notifications.length > 20_000) this.notifications.shift();
      for (const waiter of [...this.waiters]) {
        if (!waiter.predicate(observed)) continue;
        this.waiters.delete(waiter);
        clearTimeout(waiter.timer);
        waiter.resolve(observed);
      }
    }
  }

  #failAll(error) {
    for (const { reject, timer } of this.pending.values()) {
      clearTimeout(timer);
      reject(error);
    }
    this.pending.clear();
  }

  notify(method, params) {
    this.#write({ method, params });
  }

  request(method, params = {}, timeoutMs = this.timeoutMs) {
    if (!this.child?.stdin?.writable) throw new Error('app-server stdin is not writable');
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`app-server ${method} timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer, method });
      this.#write({ id, method, params });
    });
  }

  #write(message) {
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  waitFor(predicate, timeoutMs = this.timeoutMs, startIndex = 0) {
    const existing = this.notifications.slice(startIndex).findLast(predicate);
    if (existing) return Promise.resolve(existing);
    return new Promise((resolve, reject) => {
      const waiter = { predicate, resolve, reject, timer: null };
      waiter.timer = setTimeout(() => {
        this.waiters.delete(waiter);
        reject(new Error(`app-server notification timed out after ${timeoutMs}ms`));
      }, timeoutMs);
      this.waiters.add(waiter);
    });
  }

  async listModels() {
    const response = await this.request('model/list', { limit: 100 });
    return response?.data ?? [];
  }

  async readRateLimits() {
    return this.request('account/rateLimits/read', null);
  }

  async readUsage() {
    return this.request('account/usage/read', null);
  }

  async readThread(threadId) {
    const response = await this.request('thread/read', { threadId, includeTurns: true });
    if (response?.thread?.id !== threadId) throw new Error(`thread/read response did not bind ${threadId}`);
    return response.thread;
  }

  async startThread({
    model,
    cwd,
    reasoningEffort = 'high',
    fastMode = false,
    sandbox = 'read-only',
  }) {
    const started = await this.request('thread/start', {
      model,
      cwd,
      ephemeral: false,
      approvalPolicy: 'never',
      sandbox,
      serviceTier: fastMode ? 'fast' : null,
      config: { model_reasoning_effort: reasoningEffort },
    });
    return {
      threadId: started?.thread?.id,
      requestedModel: model,
      resolvedModel: started?.model ?? unavailable('thread_start_omitted_model'),
      modelProvider: started?.modelProvider ?? unavailable('thread_start_omitted_model_provider'),
      reasoningEffort: started?.reasoningEffort ?? reasoningEffort,
      serviceTier: started?.serviceTier ?? null,
    };
  }

  async forkThread({
    threadId,
    model,
    cwd,
    reasoningEffort = 'high',
    fastMode = false,
    sandbox = 'read-only',
  }) {
    const forked = await this.request('thread/fork', {
      threadId,
      model,
      cwd,
      ephemeral: false,
      approvalPolicy: 'never',
      sandbox,
      serviceTier: fastMode ? 'fast' : null,
      config: { model_reasoning_effort: reasoningEffort },
    });
    return {
      threadId: forked?.thread?.id,
      requestedModel: model,
      resolvedModel: forked?.model ?? unavailable('thread_fork_omitted_model'),
      modelProvider: forked?.modelProvider ?? unavailable('thread_fork_omitted_model_provider'),
      reasoningEffort: forked?.reasoningEffort ?? reasoningEffort,
      serviceTier: forked?.serviceTier ?? null,
    };
  }

  async resumeThread({
    threadId,
    model,
    cwd,
    reasoningEffort = 'high',
    fastMode = false,
    sandbox = 'read-only',
  }) {
    const resumed = await this.request('thread/resume', {
      threadId,
      model,
      cwd,
      approvalPolicy: 'never',
      sandbox,
      serviceTier: fastMode ? 'fast' : null,
      config: { model_reasoning_effort: reasoningEffort },
    });
    return {
      threadId: resumed?.thread?.id ?? threadId,
      requestedModel: model,
      resolvedModel: resumed?.model ?? unavailable('thread_resume_omitted_model'),
      modelProvider: resumed?.modelProvider ?? unavailable('thread_resume_omitted_model_provider'),
      reasoningEffort: resumed?.reasoningEffort ?? reasoningEffort,
      serviceTier: resumed?.serviceTier ?? null,
    };
  }

  async runTurn({ threadId, text, model, reasoningEffort = 'high', outputSchema, cwd }) {
    const sentAtMs = performance.now();
    const startIndex = this.notifications.length;
    const response = await this.request('turn/start', {
      threadId,
      input: [{ type: 'text', text }],
      model,
      effort: reasoningEffort,
      summary: 'none',
      outputSchema: outputSchema ?? null,
      cwd: cwd ?? null,
      serviceTier: null,
    });
    const turnId = response?.turn?.id;
    if (!turnId) throw new Error('turn/start response omitted turn.id');
    const completed = await this.waitFor(
      (event) => event.method === 'turn/completed' && event.params?.threadId === threadId && event.params?.turn?.id === turnId,
    );
    const completedAtMs = completed.observedAtMs;
    const events = this.notifications.slice(startIndex).filter((event) => sameTurn(event, threadId, turnId));
    const firstToken = events.find((event) =>
      event.method === 'item/agentMessage/delta' ||
      (event.method === 'item/started' && event.params?.item?.type === 'agentMessage'),
    );
    const tokenUsage = events.filter((event) => event.method === 'thread/tokenUsage/updated').at(-1)?.params?.tokenUsage;
    const reroutes = events
      .filter((event) => event.method === 'model/rerouted')
      .map((event) => ({ fromModel: event.params.fromModel, toModel: event.params.toModel, reason: event.params.reason }));
    const verification = events.filter((event) => event.method === 'model/verification').at(-1)?.params?.verifications;
    const toolCalls = events
      .filter((event) => event.method === 'item/completed' && isToolItem(event.params?.item))
      .map((event) => ({ type: event.params.item.type, tool: event.params.item.tool ?? null, status: event.params.item.status ?? null }));
    const finalText = finalAgentText(events, completed.params?.turn?.items ?? []);
    return {
      threadId,
      turnId,
      finalText,
      timing: {
        ttftMs: firstToken ? Math.max(0, firstToken.observedAtMs - sentAtMs) : unavailable('agent_message_delta_not_exposed'),
        completionMs: Math.max(0, completedAtMs - sentAtMs),
      },
      tokenUsage: tokenUsage ?? unavailable('thread_token_usage_notification_not_exposed'),
      toolCalls,
      toolCallCount: toolCalls.length,
      reroutes,
      modelVerification: verification ?? unavailable('model_verification_not_exposed'),
      turnStatus: completed.params?.turn?.status ?? 'unknown',
      turnError: completed.params?.turn?.error ?? null,
    };
  }

  async compactThread(threadId) {
    const sentAtMs = performance.now();
    const startIndex = this.notifications.length;
    const response = await this.request('thread/compact/start', { threadId });
    const responseTurnId = response?.turn?.id ?? response?.id ?? null;
    const compactionItem = await this.waitFor(
      (event) => {
        if (event.params?.threadId !== threadId || !isContextCompactionCompletion(event)) return false;
        const notificationTurnId = event.params?.turnId;
        if (!notificationTurnId || this.completedCompactionTurnIds.has(notificationTurnId)) return false;
        return responseTurnId ? notificationTurnId === responseTurnId : true;
      },
      this.timeoutMs,
      startIndex,
    );
    const turnId = compactionItem.params.turnId;
    const itemStatus = compactionItem.params?.item?.status;
    if (itemStatus && itemStatus !== 'completed') {
      throw new Error(`context compaction ${turnId} ended with item status ${itemStatus}`);
    }
    const completedTurn = await this.waitFor(
      (event) => event.method === 'turn/completed' &&
        event.params?.threadId === threadId &&
        event.params?.turn?.id === turnId,
      this.timeoutMs,
      startIndex,
    );
    const turnStatus = completedTurn.params?.turn?.status ?? 'unknown';
    const turnError = completedTurn.params?.turn?.error ?? null;
    if (turnStatus !== 'completed' || turnError !== null) {
      throw new Error(`context compaction ${turnId} ended with turn status ${turnStatus}`);
    }
    this.completedCompactionTurnIds.add(turnId);
    const events = this.notifications.slice(startIndex).filter((event) => exactSameTurn(event, threadId, turnId));
    const tokenUsage = events.filter((event) => event.method === 'thread/tokenUsage/updated').at(-1)?.params?.tokenUsage;
    const modelVerification = events.filter((event) => event.method === 'model/verification').at(-1)?.params?.verifications;
    const toolCalls = events
      .filter((event) => event.method === 'item/completed' && isToolItem(event.params?.item))
      .map((event) => ({ type: event.params.item.type, tool: event.params.item.tool ?? null, status: event.params.item.status ?? null }));
    const itemCompletionMs = Math.max(0, compactionItem.observedAtMs - sentAtMs);
    const completionMs = Math.max(0, completedTurn.observedAtMs - sentAtMs);
    return {
      turnId,
      durationMs: completionMs,
      observed: true,
      timing: { itemCompletionMs, completionMs },
      tokenUsage: tokenUsage ?? unavailable('compaction_token_usage_notification_not_exposed'),
      toolCalls: toolCalls.length ? toolCalls : unavailable('compaction_tool_notifications_not_exposed'),
      toolCallCount: toolCalls.length ? toolCalls.length : unavailable('compaction_tool_notifications_not_exposed'),
      modelVerification: modelVerification ?? unavailable('compaction_model_verification_not_exposed'),
      turnStatus,
      turnError,
    };
  }

  async close() {
    if (!this.child) return;
    this.child.stdin.end();
    if (!this.child.killed) this.child.kill('SIGTERM');
    this.child = null;
  }
}

export class RpcError extends Error {
  constructor(method, rpc) {
    super(`Codex app-server ${method} error ${rpc.code ?? 'unknown'}: ${rpc.message ?? 'unknown error'}`);
    this.name = 'RpcError';
    this.code = rpc.code;
    this.data = rpc.data;
  }
}

function sameTurn(event, threadId, turnId) {
  const params = event.params ?? {};
  return params.threadId === threadId && (params.turnId === turnId || params.turn?.id === turnId || params.turnId === undefined);
}

function exactSameTurn(event, threadId, turnId) {
  const params = event.params ?? {};
  return params.threadId === threadId && (params.turnId === turnId || params.turn?.id === turnId);
}

function isToolItem(item) {
  return ['commandExecution', 'fileChange', 'mcpToolCall', 'dynamicToolCall', 'collabAgentToolCall', 'webSearch'].includes(item?.type);
}

function isContextCompactionCompletion(event) {
  return event.method === 'item/completed' && event.params?.item?.type === 'contextCompaction';
}

function finalAgentText(events, completedItems) {
  const completed = [...events]
    .reverse()
    .find((event) => event.method === 'item/completed' && event.params?.item?.type === 'agentMessage')?.params?.item?.text;
  if (typeof completed === 'string') return completed;
  const fallback = [...completedItems].reverse().find((item) => item?.type === 'agentMessage')?.text;
  return typeof fallback === 'string' ? fallback : '';
}

export function maximumUsedPercent(snapshot) {
  const snapshots = [snapshot?.rateLimits, ...Object.values(snapshot?.rateLimitsByLimitId ?? {})].filter(Boolean);
  const values = snapshots.flatMap((item) => [item.primary?.usedPercent, item.secondary?.usedPercent]).filter(Number.isFinite);
  return values.length ? Math.max(...values) : null;
}
