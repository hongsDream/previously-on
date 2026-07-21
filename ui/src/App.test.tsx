import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { fallbackData } from './data/fallback';
import type { AiFactCandidateV1, AiFactRefreshOperationV1, AgentV1, BootstrapData, RelationshipGraphV1, TaskGroupingOperationV1 } from './types';

function liveWorkspace() {
  const data = structuredClone(fallbackData);
  data.resumeCandidate = undefined;
  return data;
}

describe('PreviouslyOn review workspace', () => {
  beforeEach(() => {
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('API unavailable')));
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('labels bundled history as untrusted display-only data', () => {
    expect(fallbackData.trust).toEqual({
      classification: 'untrusted_historical_data',
      instructionPolicy: 'display_only_never_execute',
      source: 'previously_on_local_history',
    });
  });

  it('loads the safe fallback only when bootstrap is unavailable', async () => {
    render(<App />);

    expect(await screen.findByRole('heading', { name: 'Refactor authentication boundary' })).toBeInTheDocument();
    expect(screen.getByText(/Local API unavailable · read-only sample workspace/)).toBeInTheDocument();
    expect(screen.getAllByText('Authentication boundary will be enforced in middleware layer; handlers will depend on AuthContext interface only.').length).toBeGreaterThan(0);
    expect(screen.getAllByText('864 tokens').length).toBeGreaterThan(0);
  });

  it('renders session age, rollover pressure, and Git revalidation details', async () => {
    render(<App />);

    const lineage = await screen.findByRole('region', { name: 'Codebase lineage' });
    expect(within(lineage).getAllByText('acme/api')).toHaveLength(2);
    expect(within(lineage).getByText('~/Projects/acme-app')).toBeInTheDocument();
    expect(within(lineage).getByText('feat/auth-boundary')).toBeInTheDocument();
    expect(within(lineage).getByText('3 captured')).toBeInTheDocument();
    expect(within(lineage).getByText('3 verified checkpoints')).toBeInTheDocument();
    expect(within(lineage).getAllByText('Relevant code changed')).toHaveLength(2);
    expect(within(lineage).getByText('src/auth/')).toBeInTheDocument();
    expect(await screen.findByText('New thread suggested')).toBeInTheDocument();
    expect(screen.getAllByText('7 compactions').length).toBeGreaterThan(0);
    expect(screen.getByText('81% used')).toBeInTheDocument();
    expect(screen.getAllByText('Relevant code changed').length).toBeGreaterThan(0);
    expect(screen.getByText('src/middleware/auth.ts → src/middleware/access.ts')).toBeInTheDocument();
    expect(screen.getByText('src/interfaces/legacy-auth.ts')).toBeInTheDocument();
    expect(screen.getAllByText(/last year|year ago/).length).toBeGreaterThan(0);
    expect(screen.getByText('Then')).toBeInTheDocument();
    expect(screen.getByText('Since')).toBeInTheDocument();
    expect(screen.getByText('Now')).toBeInTheDocument();
    expect(screen.getByText('Needs review')).toBeInTheDocument();
  });

  it('offers the official Codex deep link as rollover recovery', async () => {
    render(<App />);

    const link = await screen.findByRole('link', { name: 'Open in Codex' });
    expect(link).toHaveAttribute('href', 'codex://threads/thread_01HZX4AUTHBOUNDARY03');
  });

  it('keeps rendering bootstrap payloads without timeline extensions', async () => {
    const data = liveWorkspace();
    for (const checkpoint of data.checkpoints) {
      delete checkpoint.sourceThreadId;
      delete checkpoint.lastActivityAt;
      delete checkpoint.turnCount;
      delete checkpoint.compactionCount;
      delete checkpoint.contextUsage;
      delete checkpoint.continuationState;
      delete checkpoint.continuationAdvice;
      delete checkpoint.temporalRevalidation;
    }
    const pack = data.contextPacks[data.tasks[0].id];
    delete pack.temporal_revalidation;
    delete pack.current_validation;
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));

    render(<App />);

    expect(await screen.findByRole('heading', { name: 'Refactor authentication boundary' })).toBeInTheDocument();
    expect(screen.queryByText('New thread suggested')).not.toBeInTheDocument();
    expect(screen.getAllByText('Token usage unavailable').length).toBeGreaterThan(0);
    expect(screen.getByText('Then')).toBeInTheDocument();
    expect(screen.getByText('Since')).toBeInTheDocument();
    expect(screen.getByText('Now')).toBeInTheDocument();
    expect(screen.getByText('Needs review')).toBeInTheDocument();
  });

  it('keeps resume data read-only in sample mode', async () => {
    const user = userEvent.setup();
    render(<App />);

    await screen.findByText(/You have 3 uncompleted sessions/);
    const banner = screen.getByRole('region', { name: 'Resume here?' });
    await user.click(within(banner).getByRole('button', { name: 'Dismiss' }));

    await waitFor(() => expect(screen.getByText(/You have 3 uncompleted sessions/)).toBeInTheDocument());
  });

  it('filters tasks and allows selecting a matching task', async () => {
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole('heading', { name: 'Refactor authentication boundary' });
    const search = screen.getByPlaceholderText('Search tasks');
    await user.type(search, 'tenant audit');
    const taskButton = await screen.findByRole('button', { name: /Add tenant audit trail/ });
    await user.click(taskButton);

    expect(screen.getByRole('heading', { name: 'Add tenant audit trail' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'No checkpoints yet' })).toBeInTheDocument();
    const lineage = screen.getByRole('region', { name: 'Codebase lineage' });
    expect(within(lineage).getByText('feat/tenant-audit')).toBeInTheDocument();
    expect(within(lineage).getByText('~/Projects/acme-app/.worktrees/tenant-audit')).toBeInTheDocument();
    expect(within(lineage).getByText('0 captured')).toBeInTheDocument();
    expect(within(lineage).getByText('No source task IDs captured')).toBeInTheDocument();
  });

  it('shows the project overview and recent Codex sessions from primary navigation', async () => {
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole('heading', { name: 'Refactor authentication boundary' });
    await user.click(screen.getAllByRole('button', { name: 'Tasks' })[0]);
    expect(screen.getByRole('main', { name: 'Project overview' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'What this codebase remembers' })).toBeInTheDocument();
    expect(screen.getAllByText('Active tasks').length).toBeGreaterThan(0);
    expect(screen.getByText('Evidence-backed relationship graph')).toBeInTheDocument();

    const preview = screen.getByRole('button', { name: 'Preview context pack' });
    preview.focus();
    await user.keyboard('{Enter}');
    expect(screen.getByRole('heading', { name: 'Refactor authentication boundary' })).toBeInTheDocument();
    expect(screen.getByText('Needs review')).toBeInTheDocument();

    await user.click(screen.getAllByRole('button', { name: 'Sessions' })[0]);
    expect(screen.getByText('Recent sessions')).toBeInTheDocument();
    expect(screen.getByText('thread_01HZX4AUTHBOUNDARY03')).toBeInTheDocument();
    expect(screen.getByText('7 compactions · 81% context')).toBeInTheDocument();
  });

  it('connects mobile Tasks and Sessions navigation to the same project overview', async () => {
    const user = userEvent.setup();
    render(<App />);

    await screen.findByRole('heading', { name: 'Refactor authentication boundary' });
    const taskButtons = screen.getAllByRole('button', { name: 'Tasks' });
    const sessionButtons = screen.getAllByRole('button', { name: 'Sessions' });
    const evidenceButtons = screen.getAllByRole('button', { name: 'Evidence' });
    const settingsButtons = screen.getAllByRole('button', { name: 'Settings' });
    expect(taskButtons).toHaveLength(2);
    expect(sessionButtons).toHaveLength(2);
    expect(sessionButtons[1]).toBeEnabled();
    expect(settingsButtons[1]).toBeEnabled();

    await user.click(sessionButtons[1]);
    expect(screen.getByRole('main', { name: 'Project overview' })).toBeInTheDocument();
    expect(sessionButtons[1]).toHaveClass('active');

    await user.click(taskButtons[1]);
    expect(taskButtons[1]).toHaveClass('active');
    expect(screen.getByText('Evidence-backed relationship graph')).toBeInTheDocument();

    await user.click(settingsButtons[1]);
    expect(settingsButtons[1]).toHaveClass('active');
    expect(screen.getByRole('heading', { name: 'AI-assisted fact refresh' })).toBeInTheDocument();

    await user.click(evidenceButtons[1]);
    expect(screen.getByLabelText('Evidence inspector')).toHaveClass('mobile-open');
    expect(screen.getByRole('heading', { name: 'Refactor authentication boundary' })).toBeInTheDocument();
  });

  it.each([
    ['ready', 'Ready for explicit refresh'],
    ['needs_setup', 'Setup required'],
    ['unsupported', 'App Server unsupported'],
    ['blocked', 'Refresh blocked'],
  ] as const)('shows %s AI refresh capability in Settings', async (status, title) => {
    const data = liveWorkspace();
    data.aiRefreshCapability = {
      status,
      profileName: 'previously-input-only',
      reason: null,
      checkedAt: status === 'ready' ? '2025-05-21T00:00:00Z' : null,
    };
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));
    const user = userEvent.setup();
    render(<App />);

    await user.click((await screen.findAllByRole('button', { name: 'Settings' }))[0]);
    expect(screen.getByRole('main', { name: 'Settings' })).toBeInTheDocument();
    expect(screen.getByText(status.replace('_', ' '))).toBeInTheDocument();
    expect(screen.getByText(title)).toBeInTheDocument();
    expect(screen.getByText('previously-input-only')).toBeInTheDocument();
    expect(screen.getByText(/Candidate-only output/)).toBeInTheDocument();
    if (status === 'ready') {
      expect(screen.getByText('Disabled')).toBeInTheDocument();
      expect(screen.getByText('Never')).toBeInTheDocument();
    } else {
      expect(screen.getAllByText('Not verified')).toHaveLength(2);
      expect(screen.queryByText('Never')).not.toBeInTheDocument();
    }
  });

  it('switches the interface to Korean and stores the browser preference', async () => {
    const data = liveWorkspace();
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));
    const user = userEvent.setup();
    render(<App />);

    await user.click((await screen.findAllByRole('button', { name: 'Settings' }))[0]);
    await user.selectOptions(screen.getByLabelText('Language'), 'ko');

    expect(screen.getByRole('heading', { name: '설정' })).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: '작업' }).length).toBeGreaterThan(0);
    expect(screen.getByRole('heading', { name: 'AI 보조 사실 새로고침' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /Refactor authentication boundary/ }));
    expect(screen.getByText('작업 무결성')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '세션 정리' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '로컬 에이전트' })).toBeInTheDocument();
    expect(screen.queryByText('Task integrity')).not.toBeInTheDocument();
    expect(screen.queryByText('Session grouping')).not.toBeInTheDocument();
    expect(document.documentElement).toHaveAttribute('lang', 'ko');
    expect(JSON.parse(localStorage.getItem('previously-on:preferences:v1') ?? '{}')).toEqual({
      schemaVersion: 1,
      language: 'ko',
    });
  });

  it.each(['needs_setup', 'unsupported', 'blocked'] as const)('keeps Refresh facts disabled when capability is %s', async (status) => {
    const data = liveWorkspace();
    data.aiRefreshCapability = { status, profileName: 'previously-input-only', reason: `${status} reason` };
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));
    render(<App />);

    expect(await screen.findByRole('button', { name: 'Refresh facts' })).toBeDisabled();
    expect(screen.getByText(`${status} reason`)).toBeInTheDocument();
  });

  it('polls a user-started fact refresh and supports edit, accept, and reject review without creating Evidence', async () => {
    const data = liveWorkspace();
    data.aiRefreshCapability = { status: 'ready', profileName: 'previously-input-only', reason: null };
    data.factRefreshOperations = [];
    const pending = factRefreshOperation(data.tasks[0].id, 'pending');
    const completed = factRefreshOperation(data.tasks[0].id, 'completed');
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === '/api/bootstrap') return { ok: true, json: async () => data };
      if (path.endsWith(`/tasks/${data.tasks[0].id}/fact-refresh`)) return { ok: true, status: 202, json: async () => pending };
      if (path === `/api/fact-refresh/${pending.operationId}`) return { ok: true, json: async () => completed };
      if (path.includes('/candidates/')) {
        const body = JSON.parse(String(init?.body)) as { decision: 'accept' | 'reject'; content?: string; kind?: string };
        const candidate = completed.candidates.find((item) => path.endsWith(item.id))!;
        return {
          ok: true,
          json: async () => ({
            ok: true,
            candidate: { ...candidate, content: body.content ?? candidate.content, kind: body.kind ?? candidate.kind, status: body.decision === 'accept' ? 'accepted' : 'rejected' },
            ...(body.decision === 'accept' ? {
              fact: {
                id: `fact-from-${candidate.id}`,
                taskId: data.tasks[0].id,
                kind: body.kind ?? candidate.kind,
                content: body.content ?? candidate.content,
                lifecycle: 'candidate',
                updatedAt: '2025-05-21T00:01:00Z',
                evidenceIds: [],
                relatedFiles: [],
                mixedProvenance: false,
                provenanceSessionIds: [],
              },
            } : {}),
          }),
        };
      }
      throw new Error(`unexpected request ${path}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'Refresh facts' }));
    await waitFor(() => expect(fetchMock.mock.calls.some(([input]) => String(input) === `/api/fact-refresh/${pending.operationId}`)).toBe(true), { timeout: 3_000 });
    expect(await screen.findByRole('heading', { name: 'Fact candidates' })).toBeInTheDocument();
    expect(screen.getAllByText('Unavailable', { selector: 'dd' })).toHaveLength(4);
    expect(screen.getByText(`Existing fact ${completed.candidates[0].factId}`)).toBeInTheDocument();

    await user.click(screen.getAllByRole('button', { name: 'Edit' })[0]);
    const candidateText = screen.getByLabelText('Candidate text');
    await user.clear(candidateText);
    await user.type(candidateText, 'Edited candidate text for review.');
    await user.selectOptions(screen.getByLabelText('Fact kind'), 'constraint');
    await user.click(screen.getByRole('button', { name: 'Accept as Fact Candidate' }));
    await waitFor(() => {
      const reviewCall = fetchMock.mock.calls.find(([input]) => String(input).endsWith(completed.candidates[0].id));
      expect(JSON.parse(String(reviewCall?.[1]?.body))).toEqual({ decision: 'accept', content: 'Edited candidate text for review.', kind: 'constraint' });
    });
    expect(await screen.findByText('Fact Candidate')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'Reject' }));
    await waitFor(() => expect(fetchMock.mock.calls.some(([input, init]) => String(input).endsWith(completed.candidates[1].id) && JSON.parse(String(init?.body)).decision === 'reject')).toBe(true));
    expect(screen.queryByText(/model output.*Evidence/i)).not.toBeInTheDocument();
  });

  it('aborts an in-flight fact refresh poll when the task surface unmounts', async () => {
    const data = liveWorkspace();
    data.aiRefreshCapability = { status: 'ready', profileName: 'previously-input-only', reason: null };
    data.factRefreshOperations = [factRefreshOperation(data.tasks[0].id, 'pending')];
    let pollSignal: AbortSignal | undefined;
    const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === '/api/bootstrap') return Promise.resolve({ ok: true, json: async () => data });
      if (path.startsWith('/api/fact-refresh/')) {
        pollSignal = init?.signal ?? undefined;
        return new Promise(() => undefined);
      }
      throw new Error(`unexpected request ${path}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByRole('button', { name: 'Refreshing…' })).toBeDisabled();
    await waitFor(() => expect(pollSignal).toBeDefined(), { timeout: 3_000 });
    await user.click((await screen.findAllByRole('button', { name: 'Settings' }))[0]);
    await waitFor(() => expect(pollSignal?.aborted).toBe(true));
  });

  it('renders task-local agent ancestry with direct Codex links and Copy ID fallback', async () => {
    const data = liveWorkspace();
    data.agents = agentLineage(data.tasks[0].id);
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText');
    render(<App />);

    const tree = await screen.findByRole('tree', { name: /Agents observed for Refactor authentication boundary/ });
    expect(within(tree).getByText('Primary implementation')).toBeInTheDocument();
    expect(within(tree).getByText('UI verification')).toBeInTheDocument();
    expect(within(tree).getByText('degraded')).toBeInTheDocument();
    expect(within(tree).getByText('unlinked')).toBeInTheDocument();
    expect(within(tree).getByText('Parent task was not returned by the local App Server.')).toBeInTheDocument();
    const openLink = within(tree).getByRole('link', { name: `Open Codex task ${data.agents[1].threadId}` });
    expect(openLink).toHaveAttribute('href', `codex://threads/${data.agents[1].threadId}`);

    await user.click(within(tree).getByRole('button', { name: `Copy Codex task ID ${data.agents[1].threadId}` }));
    expect(writeText).toHaveBeenCalledWith(data.agents[1].threadId);
    expect(within(tree).getByRole('button', { name: `Copy Codex task ID ${data.agents[1].threadId}` })).toHaveTextContent('Copied');
  });

  it('shows explicit agent graph nodes and parent/task relationships in the semantic list fallback', async () => {
    const data = liveWorkspace();
    const graph = relationshipGraph(data.tasks[0].id);
    graph.nodes.push({ id: 'agent-ui', kind: 'agent', label: 'UI verification', taskId: data.tasks[0].id });
    graph.edges.push({
      id: 'edge-agent-task',
      kind: 'agent-worked-on-task',
      from: 'agent-ui',
      to: data.tasks[0].id,
      provenanceIds: ['agent-observation-ui'],
      sourceKind: 'agent_observation',
      observedAt: '2025-05-21T00:00:00Z',
      verified: true,
    });
    data.graphSummary = { nodeCount: graph.nodes.length, edgeCount: graph.edges.length, verifiedEdgeCount: graph.edges.length };
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => String(input) === '/api/bootstrap'
      ? { ok: true, json: async () => data }
      : { ok: true, json: async () => graph }));
    const user = userEvent.setup();
    render(<App />);

    await user.click((await screen.findAllByRole('button', { name: 'Tasks' }))[0]);
    await user.click(await screen.findByRole('button', { name: 'List' }));
    const table = await screen.findByRole('table', { name: 'Explicit relationship edges and provenance' });
    expect(within(table).getByText('agent-worked-on-task')).toBeInTheDocument();
    expect(within(table).getByText('agent_observation')).toBeInTheDocument();
    expect(screen.getAllByText('UI verification').length).toBeGreaterThan(0);
  });

  it('edits task title, goal, and lifecycle with the deterministic suggestion contract', async () => {
    let data = liveWorkspace();
    data.tasks[0].titleSuggestion = { value: 'Verified authentication boundary', source: 'goal' };
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === '/api/bootstrap') return { ok: true, json: async () => data };
      if (path === `/api/tasks/${data.tasks[0].id}` && init?.method === 'PATCH') {
        const update = JSON.parse(String(init.body));
        data = {
          ...data,
          tasks: data.tasks.map((task) => task.id === data.tasks[0].id ? { ...task, ...update, updatedAt: '2025-05-21T00:00:00Z' } : task),
        };
        return { ok: true, json: async () => ({ ok: true }) };
      }
      throw new Error(`unexpected request ${path}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'Edit task' }));
    const editor = screen.getByRole('form', { name: /Edit task Refactor authentication boundary/ });
    expect(screen.getByText('Source: verified goal first line')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Use suggestion' }));
    const goal = within(editor).getByLabelText('Goal');
    await user.clear(goal);
    await user.type(goal, 'Keep authentication at the verified middleware boundary.');
    await user.selectOptions(within(editor).getByLabelText('Status'), 'completed');
    await user.click(screen.getByRole('button', { name: 'Save task' }));

    await waitFor(() => {
      const mutation = fetchMock.mock.calls.find(([input, init]) => String(input).startsWith('/api/tasks/') && init?.method === 'PATCH');
      expect(JSON.parse(String(mutation?.[1]?.body))).toEqual({
        title: 'Verified authentication boundary',
        goal: 'Keep authentication at the verified middleware boundary.',
        status: 'completed',
      });
    });
    expect(await screen.findByRole('heading', { name: 'Verified authentication boundary' })).toBeInTheDocument();
    expect(screen.getByText('completed')).toBeInTheDocument();
  });

  it('invalidates stale grouping previews, refreshes task-scoped projections, and appends undo history', async () => {
    let data = liveWorkspace();
    const sourceTaskId = data.tasks[0].id;
    const targetTaskId = data.tasks[1].id;
    const movedSessionId = 'session-auth-2';
    data.facts[1].provenanceSessionIds = ['session-auth-2', 'session-auth-3'];
    let latestOperation: typeof data.taskGroupingOperations[number] | null = null;
    const original = structuredClone(data);

    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === '/api/bootstrap') return { ok: true, json: async () => data };
      if (path === '/api/task-grouping/preview') {
        const request = JSON.parse(String(init?.body));
        latestOperation = groupingOperation(request.operationId, request.targetTaskId, sourceTaskId, movedSessionId, data);
        return {
          ok: true,
          json: async () => ({
            operation: latestOperation,
            affectedSessions: latestOperation!.sessionMoves,
            affectedFacts: latestOperation!.factImpacts,
            counts: { sessions: 1, factsMoved: 1, factsMixed: 1 },
          }),
        };
      }
      if (path === '/api/task-grouping') {
        const request = JSON.parse(String(init?.body));
        latestOperation = groupingOperation(request.operationId, request.targetTaskId, sourceTaskId, movedSessionId, data);
        data = {
          ...data,
          tasks: data.tasks.map((task) => task.id === sourceTaskId
            ? { ...task, checkpointIds: task.checkpointIds.filter((id) => id !== 'checkpoint-2') }
            : task.id === request.targetTaskId ? { ...task, checkpointIds: [...task.checkpointIds, 'checkpoint-2'] } : task),
          sessions: data.sessions.map((session) => session.id === movedSessionId ? { ...session, taskId: request.targetTaskId } : session),
          facts: data.facts.map((fact) => fact.id === 'fact-auth-boundary'
            ? { ...fact, taskId: request.targetTaskId }
            : fact.id === 'fact-tenant-isolation' ? { ...fact, mixedProvenance: true } : fact),
          taskGroupingOperations: [latestOperation],
        };
        return { ok: true, json: async () => ({ ok: true, operation: latestOperation }) };
      }
      if (path === `/api/task-grouping/${latestOperation?.operationId}/undo`) {
        const inverse = { ...latestOperation!, operationId: 'undo-operation-1', action: 'undo' as const, inverseOf: latestOperation!.operationId, occurredAt: '2025-05-21T00:02:00Z' };
        data = { ...original, taskGroupingOperations: [latestOperation!, inverse] };
        return { ok: true, json: async () => ({ ok: true, operation: inverse }) };
      }
      throw new Error(`unexpected request ${path}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'Organize sessions' }));
    await user.click(screen.getByRole('checkbox', { name: /thread_01HZX4AUTHBOUNDARY02/ }));
    await user.selectOptions(screen.getByLabelText('Task'), targetTaskId);
    await user.click(screen.getByRole('button', { name: 'Preview impact' }));
    expect(await screen.findByRole('heading', { name: 'Impact preview' })).toBeInTheDocument();
    expect(screen.getByText(/mixed provenance · not duplicated/)).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText('Task'), data.tasks[2].id);
    expect(screen.queryByRole('button', { name: 'Confirm move' })).not.toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText('Task'), targetTaskId);
    await user.click(screen.getByRole('button', { name: 'Preview impact' }));
    await user.click(await screen.findByRole('button', { name: 'Confirm move' }));

    expect(await screen.findByText('Operation history')).toBeInTheDocument();
    expect(screen.queryByLabelText('Evidence inspector')).not.toBeInTheDocument();
    const undo = screen.getByRole('button', { name: `Undo grouping operation ${latestOperation!.operationId}` });
    await user.click(undo);
    await waitFor(() => expect(fetchMock.mock.calls.some(([input]) => String(input).endsWith(`/${latestOperation!.operationId}/undo`))).toBe(true));
    expect(await screen.findByText('Undone')).toBeInTheDocument();
  });

  it('renders the explicit relationship graph with a keyboard-accessible semantic table fallback', async () => {
    const data = liveWorkspace();
    data.graphSummary = { nodeCount: 3, edgeCount: 2, verifiedEdgeCount: 2 };
    const graph = relationshipGraph(data.tasks[0].id);
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const path = String(input);
      if (path === '/api/bootstrap') return { ok: true, json: async () => data };
      if (path.startsWith('/api/graph?repository=acme%2Fapi')) return { ok: true, json: async () => graph };
      throw new Error(`unexpected request ${path}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByRole('button', { name: 'All tasks' })).toBeInTheDocument();
    await user.click((await screen.findAllByRole('button', { name: 'Tasks' }))[0]);
    expect(await screen.findByRole('img', { name: /Relationship graph with 3 nodes and 2 edges/ })).toBeInTheDocument();
    const listView = screen.getByRole('button', { name: 'List' });
    listView.focus();
    await user.keyboard('{Enter}');
    expect(listView).toHaveAttribute('aria-pressed', 'true');
    const table = await screen.findByRole('table', { name: 'Explicit relationship edges and provenance' });
    expect(within(table).getByText('task-has-session')).toBeInTheDocument();
    expect(within(table).getByText('canonical_event')).toBeInTheDocument();
    expect(within(table).getByText('event-task-session')).toBeInTheDocument();
    expect(within(table).queryByRole('columnheader', { name: 'Verified' })).not.toBeInTheDocument();
    expect(screen.queryByText(/similarity/i)).not.toBeInTheDocument();
  });

  it('defensively hides legacy graph edges marked unverified', async () => {
    const data = liveWorkspace();
    const graph = relationshipGraph(data.tasks[0].id);
    graph.edges[1].verified = false;
    data.graphSummary = { nodeCount: 3, edgeCount: 2, verifiedEdgeCount: 1 };
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => String(input) === '/api/bootstrap'
      ? { ok: true, json: async () => data }
      : { ok: true, json: async () => graph }));
    const user = userEvent.setup();
    render(<App />);

    await user.click((await screen.findAllByRole('button', { name: 'Tasks' }))[0]);
    expect(await screen.findByRole('img', { name: /Relationship graph with 3 nodes and 1 edges/ })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'List' }));
    const table = await screen.findByRole('table', { name: 'Explicit relationship edges and provenance' });
    expect(within(table).getByText('task-has-session')).toBeInTheDocument();
    expect(within(table).queryByText('session-changed-file')).not.toBeInTheDocument();
    expect(within(table).queryByText('Unverified')).not.toBeInTheDocument();
  });

  it('defaults the relationship graph to the list fallback on narrow viewports', async () => {
    vi.stubGlobal('matchMedia', vi.fn().mockImplementation((query: string) => ({
      matches: query === '(max-width: 900px)',
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })));
    const data = liveWorkspace();
    data.graphSummary = { nodeCount: 3, edgeCount: 2, verifiedEdgeCount: 2 };
    const graph = relationshipGraph(data.tasks[0].id);
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => String(input) === '/api/bootstrap'
      ? { ok: true, json: async () => data }
      : { ok: true, json: async () => graph }));
    const user = userEvent.setup();
    render(<App />);

    await user.click((await screen.findAllByRole('button', { name: 'Tasks' }))[0]);
    expect(await screen.findByRole('table', { name: 'Explicit relationship edges and provenance' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'List' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.queryByRole('img', { name: /Relationship graph with/ })).not.toBeInTheDocument();
  });

  it('edits fact memory and excludes its source session through the local API', async () => {
    const data = liveWorkspace();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === '/api/bootstrap') return { ok: true, json: async () => data };
      if (path.startsWith('/api/facts/')) {
        const body = JSON.parse(String(init?.body));
        return { ok: true, json: async () => ({ ok: true, text: body.content, status: body.status, updatedAt: '2025-05-20T00:00:00Z', deprecatedAfterCommit: body.deprecatedAfterCommit }) };
      }
      if (path.startsWith('/api/sessions/')) {
        const body = JSON.parse(String(init?.body));
        return { ok: true, json: async () => ({ ok: true, sessionId: 'session-auth-2', excluded: body.excluded }) };
      }
      throw new Error(`unexpected request ${path}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'Edit' }));
    const factText = screen.getByLabelText('Fact text');
    await user.clear(factText);
    await user.type(factText, 'Handlers depend only on the verified AuthContext boundary.');
    await user.type(screen.getByLabelText(/Deprecate after Git commit/), 'abcdef1');
    await user.click(screen.getByRole('button', { name: 'Save memory' }));
    await waitFor(() => expect(fetchMock.mock.calls.some(([input, init]) => {
      if (!String(input).startsWith('/api/facts/') || init?.method !== 'PATCH') return false;
      return JSON.parse(String(init.body)).deprecatedAfterCommit === 'abcdef1';
    })).toBe(true));

    await user.click(screen.getByRole('button', { name: 'Exclude session' }));
    await waitFor(() => expect(fetchMock.mock.calls.some(([input, init]) => String(input) === '/api/sessions/session-auth-2' && JSON.parse(String(init?.body)).excluded === true)).toBe(true));
    expect(await screen.findByText('This session is excluded from future Context Packs.')).toBeInTheDocument();
  });

  it('does not mutate facts in fallback mode', async () => {
    const user = userEvent.setup();
    render(<App />);

    const pinButtons = await screen.findAllByRole('button', { name: 'Pin' });
    await user.click(pinButtons.at(-1)!);

    expect(pinButtons.at(-1)).not.toHaveClass('active');
  });

  it('renders a connected empty workspace when the API has no tasks', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        repository: { name: 'new-repo', path: '/tmp/My Repo', branch: 'main', connected: true, state: 'registered-empty', captureHealth: 'good' },
        tasks: [],
        checkpoints: [],
        facts: [],
        evidence: [],
        contextPacks: {},
      }),
    });
    vi.stubGlobal('fetch', fetchMock);

    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, 'writeText');
    render(<App />);

    expect(await screen.findByRole('heading', { name: 'new-repo is connected' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Regression contracts' })).toBeInTheDocument();
    expect(screen.getByText('Readiness unavailable')).toBeInTheDocument();
    expect(screen.getByText(/Start one captured Codex session/)).toBeInTheDocument();
    expect(screen.getByText("previously run codex --repo '/tmp/My Repo' --")).toBeInTheDocument();
    expect(screen.getByText('previously doctor')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Preview context pack' })).toBeDisabled();
    expect(screen.getAllByRole('button', { name: 'Evidence' }).every((button) => button.hasAttribute('disabled'))).toBe(true);
    expect(screen.queryByText(/sample workspace/)).not.toBeInTheDocument();

    const copyRun = screen.getByRole('button', { name: 'Copy Start a captured Codex session' });
    copyRun.focus();
    await user.keyboard('{Enter}');
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("previously run codex --repo '/tmp/My Repo' --"));
    await user.click(screen.getByRole('button', { name: 'Refresh status' }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));

    const settings = screen.getAllByRole('button', { name: 'Settings' });
    await user.click(settings.at(-1)!);
    expect(screen.getByRole('main', { name: 'Settings' })).toBeInTheDocument();
    const tasks = screen.getAllByRole('button', { name: 'Tasks' });
    await user.click(tasks.at(-1)!);
    expect(screen.getByRole('heading', { name: 'new-repo is connected' })).toBeInTheDocument();
  });

  it('connects Codex only after explicit setup consent and then requires a restart', async () => {
    const data = liveWorkspace();
    data.repository = {
      name: 'No repository',
      path: '',
      branch: 'detached',
      connected: false,
      state: 'unregistered',
      captureHealth: 'offline',
    };
    data.tasks = [];
    data.checkpoints = [];
    data.facts = [];
    data.evidence = [];
    data.sessions = [];
    data.contracts = [];
    data.contractCandidates = [];
    data.contractEvaluation = null;
    data.contractEvaluations = [];
    data.contextPacks = {};
    const registered = structuredClone(data);
    registered.repository = {
      name: 'My Repo',
      path: '/tmp/My Repo',
      branch: 'main',
      connected: true,
      state: 'registered-empty',
      captureHealth: 'good',
    };
    let bootstrapReads = 0;
    const fetchMock = vi.fn().mockImplementation((input, init) => {
      const path = String(input);
      if (path === '/api/setup/codex') {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            ok: true,
            repositoryPath: '/tmp/My Repo',
            restartRequired: true,
            doctor: {
              healthy: true,
              checks: [
                { name: 'git', ok: true, detail: 'git version 2.50' },
                { name: 'Codex hooks', ok: true, detail: 'managed entry present' },
              ],
            },
          }),
        });
      }
      if (path === '/api/bootstrap') {
        bootstrapReads += 1;
        return Promise.resolve({ ok: true, json: async () => bootstrapReads === 1 ? data : registered });
      }
      throw new Error(`unexpected request ${path} ${String(init?.method)}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();

    render(<App />);

    expect(await screen.findByRole('heading', { name: 'Connect Codex to your repository' })).toBeInTheDocument();
    expect(screen.getAllByText('Not registered').length).toBeGreaterThan(0);
    expect(screen.queryByText('Connected')).not.toBeInTheDocument();
    expect(screen.queryByText('Ready to complete')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Preview context pack' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'New candidate' })).toBeDisabled();

    await user.click(screen.getByRole('button', { name: 'More options' }));
    expect(screen.getByRole('menuitem', { name: 'Export JSON' })).toBeDisabled();
    expect(screen.getByRole('menuitem', { name: 'Delete repository data' })).toBeDisabled();

    const connect = screen.getByRole('button', { name: 'Connect Codex' });
    expect(connect).toBeDisabled();
    await user.type(screen.getByLabelText('Repository path'), 'relative/repo');
    expect(screen.getByText('Enter an absolute path beginning with /.')).toBeInTheDocument();
    expect(connect).toBeDisabled();
    await user.clear(screen.getByLabelText('Repository path'));
    await user.type(screen.getByLabelText('Repository path'), '/tmp/My Repo');
    expect(connect).toBeDisabled();
    await user.click(screen.getByRole('checkbox', { name: /I approve updating my local Codex configuration/ }));
    expect(connect).toBeEnabled();
    connect.focus();
    await user.keyboard('{Enter}');
    expect(await screen.findByRole('heading', { name: 'Codex connection installed' })).toBeInTheDocument();
    expect(screen.getByText('Local checks passed')).toBeInTheDocument();
    expect(screen.getByText('Restart Codex once')).toBeInTheDocument();
    await waitFor(() => expect(fetchMock).toHaveBeenCalledWith('/api/setup/codex', expect.objectContaining({
      method: 'POST',
      body: JSON.stringify({ repositoryPath: '/tmp/My Repo', confirmed: true }),
    })));

    await user.click(screen.getByRole('button', { name: 'I restarted Codex · Continue' }));
    expect(await screen.findByRole('heading', { name: 'My Repo is connected' })).toBeInTheDocument();
    expect(screen.getByText("previously run codex --repo '/tmp/My Repo' --")).toBeInTheDocument();
  });

  it('keeps first-run setup recoverable when the repository cannot be registered', async () => {
    const data = liveWorkspace();
    data.repository = {
      name: 'No repository',
      path: '',
      branch: 'detached',
      connected: false,
      state: 'unregistered',
      captureHealth: 'offline',
    };
    data.tasks = [];
    data.checkpoints = [];
    data.facts = [];
    data.evidence = [];
    data.sessions = [];
    data.contracts = [];
    data.contractCandidates = [];
    data.contractEvaluation = null;
    data.contractEvaluations = [];
    data.contextPacks = {};
    vi.stubGlobal('fetch', vi.fn().mockImplementation((input) => Promise.resolve(
      String(input) === '/api/setup/codex'
        ? { ok: false, status: 400, json: async () => ({ error: 'repository is not a Git work tree' }) }
        : { ok: true, json: async () => data },
    )));
    const user = userEvent.setup();

    render(<App />);
    await screen.findByRole('heading', { name: 'Connect Codex to your repository' });
    await user.type(screen.getByLabelText('Repository path'), '/tmp/not-a-repo');
    await user.click(screen.getByRole('checkbox', { name: /I approve updating my local Codex configuration/ }));
    await user.click(screen.getByRole('button', { name: 'Connect Codex' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('repository is not a Git work tree');
    expect(screen.getByRole('button', { name: 'Connect Codex' })).toBeEnabled();
  });

  it('disables pack and evidence controls when an active task has neither', async () => {
    const data = liveWorkspace();
    data.contextPacks = {};
    data.evidence = [];
    data.facts = [];
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));

    render(<App />);

    expect(await screen.findByRole('heading', { name: 'Refactor authentication boundary' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Preview context pack' })).toBeDisabled();
    expect(screen.getAllByRole('button', { name: 'Evidence' }).every((button) => button.hasAttribute('disabled'))).toBe(true);
    expect(screen.queryByLabelText('Evidence inspector')).not.toBeInTheDocument();
  });

  it('renders the actual evidence sequence for arbitrary evidence IDs', async () => {
    const data = liveWorkspace();
    data.evidence[1].id = 'evidence-arbitrary-42';
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));
    const user = userEvent.setup();

    render(<App />);

    await screen.findByLabelText('Evidence item');
    await user.selectOptions(screen.getByLabelText('Evidence item'), 'evidence-arbitrary-42');
    expect(await screen.findByText('Evidence ID: evidence-arbitrary-42')).toBeInTheDocument();
    expect(screen.getByText('E-2')).toBeInTheDocument();
    expect(document.body).not.toHaveTextContent('E-2-LMN');
  });

  it.each([
    ['active', 'good', false],
    ['degraded', 'degraded', true],
  ] as const)('renders the explicit %s repository state', async (state, captureHealth, showsWarning) => {
    const data = liveWorkspace();
    data.repository.state = state;
    data.repository.captureHealth = captureHealth;
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));

    render(<App />);

    expect((await screen.findAllByRole('button', { name: `Repository state: ${state === 'active' ? 'Active' : 'Degraded'}` })).length).toBeGreaterThan(0);
    if (showsWarning) {
      expect(screen.getByText(/Capture degraded · review missing evidence/)).toBeInTheDocument();
    } else {
      expect(screen.queryByText(/Capture degraded · review missing evidence/)).not.toBeInTheDocument();
    }
    expect(screen.getByText('Local device')).toBeInTheDocument();
    expect(screen.getByText('· No cloud account')).toBeInTheDocument();
    expect(document.body).not.toHaveTextContent('jdoe');
  });

  it('creates a manual candidate with camelCase argv fields', async () => {
    const data = liveWorkspace();
    data.contractCandidates = [];
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      if (String(input) === '/api/bootstrap') return { ok: true, json: async () => data };
      const draft = JSON.parse(String(init?.body));
      return {
        ok: true,
        json: async () => ({
          candidate: {
            schemaVersion: 1,
            id: 'manual-candidate-created',
            repositoryId: data.repository.path,
            status: 'pending',
            origin: {
              fixedAtCommit: 'abcdef12',
              recordedAt: '2025-05-20T00:00:00Z',
              evidenceSha256: '9c56cc51b374c3ba189210d5b6d4bf57790d351c96c47c0211e17dab5a23999c',
            },
            evidenceKind: 'manual',
            evidenceSha256: '9c56cc51b374c3ba189210d5b6d4bf57790d351c96c47c0211e17dab5a23999c',
            createdAt: '2025-05-20T00:00:00Z',
            updatedAt: '2025-05-20T00:00:00Z',
            ...draft,
          },
        }),
      };
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'New candidate' }));
    await user.type(screen.getByLabelText('Title'), 'Preserve billing idempotency');
    await user.type(screen.getByLabelText('Invariant'), 'A webhook event is applied at most once.');
    await user.type(screen.getByLabelText('Git path'), 'src/billing/');
    await user.type(screen.getByLabelText('Test name'), 'Billing webhook regression');
    await user.type(screen.getByLabelText('Program'), 'cargo');
    fireEvent.change(screen.getByLabelText('Arguments (one per line)'), { target: { value: 'test\nbilling_webhook' } });
    await user.click(screen.getByRole('button', { name: 'Create candidate' }));

    await waitFor(() => {
      const mutation = fetchMock.mock.calls.find(([input]) => String(input) === '/api/contract-candidates');
      expect(mutation?.[1]?.method).toBe('POST');
      expect(JSON.parse(String(mutation?.[1]?.body))).toMatchObject({
        title: 'Preserve billing idempotency',
        invariant: 'A webhook event is applied at most once.',
        impactSelectors: [{ path: { kind: 'exact', value: 'src/billing/' }, symbols: [] }],
        requiredTests: [{
          name: 'Billing webhook regression',
          program: 'cargo',
          args: ['test', 'billing_webhook'],
          workingDirectory: '.',
          timeoutSeconds: 900,
        }],
      });
    });
    expect(await screen.findByRole('heading', { name: 'Preserve billing idempotency' })).toBeInTheDocument();
  });

  it('edits and approves a pending candidate, then supersedes an active contract', async () => {
    const data = liveWorkspace();
    const pending = data.contractCandidates[0];
    const approvedContract = {
      schemaVersion: 1 as const,
      id: 'new-auth-contract',
      title: 'Authentication remains middleware-owned',
      invariant: pending.invariant,
      status: 'active' as const,
      impactSelectors: pending.impactSelectors,
      requiredTests: pending.requiredTests,
      origin: {
        fixedAtCommit: 'abcdef12',
        recordedAt: '2025-05-20T00:00:00Z',
        evidenceSha256: '88d4266fd4e6338d13b845fcf289579d209c897823b9217da3e161936d9c158e',
      },
    };
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === '/api/bootstrap') return { ok: true, json: async () => data };
      if (path.endsWith('/approve')) return { ok: true, json: async () => ({ contract: approvedContract }) };
      if (path.endsWith('/supersede')) {
        return { ok: true, json: async () => ({
          contract: { ...data.contracts[0], status: 'superseded', supersededBy: approvedContract.id },
        }) };
      }
      const draft = JSON.parse(String(init?.body));
      return { ok: true, json: async () => ({ candidate: { ...pending, ...draft } }) };
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: `Edit ${pending.title}` }));
    const title = screen.getByLabelText('Title');
    await user.clear(title);
    await user.type(title, approvedContract.title);
    await user.click(screen.getByRole('button', { name: 'Save candidate' }));
    await waitFor(() => expect(fetchMock.mock.calls.some(([input, init]) => String(input).includes(`/api/contract-candidates/${pending.id}`) && init?.method === 'PATCH')).toBe(true));

    await user.click(await screen.findByRole('button', { name: `Approve ${approvedContract.title}` }));
    await waitFor(() => expect(fetchMock.mock.calls.some(([input]) => String(input).endsWith(`/${pending.id}/approve`))).toBe(true));
    expect(await screen.findByRole('heading', { name: approvedContract.title })).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText(`Replacement for ${data.contracts[0].title}`), approvedContract.id);
    await user.click(screen.getByRole('button', { name: `Supersede ${data.contracts[0].title}` }));
    await waitFor(() => {
      const mutation = fetchMock.mock.calls.find(([input]) => String(input).endsWith(`/${data.contracts[0].id}/supersede`));
      expect(JSON.parse(String(mutation?.[1]?.body))).toEqual({ supersededBy: approvedContract.id });
    });
    expect(screen.getByText(`Superseded by ${approvedContract.id}`)).toBeInTheDocument();
  });

  it('shows passed, failed, missing, and stale required test states', async () => {
    const data = liveWorkspace();
    data.contractEvaluation!.requiredTests = (['passed', 'failed', 'missing', 'stale'] as const).map((status, index) => ({
      contractId: data.contracts[0].id,
      testId: `test-${status}`,
      name: `Required test ${index + 1}`,
      program: 'cargo',
      args: ['test', `test_${status}`],
      workingDirectory: '.',
      timeoutSeconds: 900,
      state: status,
    }));
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));
    render(<App />);

    expect(await screen.findByText('Not ready to complete')).toBeInTheDocument();
    for (const status of ['passed', 'failed', 'missing', 'stale']) {
      expect(screen.getAllByText(status).find((item) => item.classList.contains(`test-state-${status}`))).toBeDefined();
    }
    expect(screen.getByText('Prefix src/middleware/ matched src/middleware/access.ts')).toBeInTheDocument();
  });

  it('shows Contract readiness for the selected task instead of the newest repository evaluation', async () => {
    const data = liveWorkspace();
    const blocked = structuredClone(data.contractEvaluation!);
    blocked.taskId = data.tasks[0].id;
    blocked.readiness = 'contract_blocked';
    const ready = structuredClone(blocked);
    ready.id = 'evaluation-tenant-audit';
    ready.taskId = data.tasks[1].id;
    ready.readiness = 'ready';
    ready.requiredTests = ready.requiredTests.map((test) => ({ ...test, state: 'passed' as const }));
    data.contractEvaluation = blocked;
    data.contractEvaluations = [blocked, ready];
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: async () => data }));
    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByText('Not ready to complete')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: /Add tenant audit trail/ }));
    expect(await screen.findByText('Ready to complete')).toBeInTheDocument();
    expect(screen.queryByText('Not ready to complete')).not.toBeInTheDocument();
  });

  it('rolls back an optimistic candidate edit when the API rejects it', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const data = liveWorkspace();
    const pending = data.contractCandidates[0];
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      if (String(input) === '/api/bootstrap') return { ok: true, json: async () => data };
      return { ok: false, status: 500, json: async () => ({ error: 'failed' }) };
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: `Edit ${pending.title}` }));
    const title = screen.getByLabelText('Title');
    await user.clear(title);
    await user.type(title, 'Unsafe optimistic title');
    await user.click(screen.getByRole('button', { name: 'Save candidate' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('PreviouslyOn API is unavailable');
    await waitFor(() => expect(screen.getByRole('heading', { name: pending.title })).toBeInTheDocument());
  });

  it('renders the actual selected context pack and persists explicit replacement relations', async () => {
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      void init;
      const path = String(input);
      if (path === '/api/bootstrap') {
        return { ok: true, json: async () => liveWorkspace() };
      }
      return { ok: true, json: async () => ({ ok: true }) };
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByText('2 files')).toBeInTheDocument();
    expect(screen.queryByText('+ 6 more')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Supersede' }));
    await user.click(screen.getByRole('button', { name: 'Apply' }));

    await waitFor(() => {
      const mutation = fetchMock.mock.calls.find(([input]) => String(input).includes('/api/facts/'));
      expect(mutation).toBeDefined();
      expect(JSON.parse(String(mutation?.[1]?.body))).toEqual({
        status: 'superseded',
        supersedesFactId: 'fact-tenant-isolation',
      });
    });
  });

  it('rolls back optimistic fact changes when the local API rejects the mutation', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      if (String(input) === '/api/bootstrap') {
        return { ok: true, json: async () => liveWorkspace() };
      }
      return { ok: false, status: 500, json: async () => ({ error: 'failed' }) };
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    render(<App />);

    const pin = await screen.findByRole('button', { name: 'Pin' });
    await user.click(pin);

    expect(await screen.findByRole('alert')).toHaveTextContent('PreviouslyOn API is unavailable');
    await waitFor(() => expect(pin).not.toHaveClass('active'));
  });
});

function groupingOperation(operationId: string, targetTaskId: string, sourceTaskId: string, sessionId: string, data: BootstrapData): TaskGroupingOperationV1 {
  return {
    schemaVersion: 1,
    operationId,
    repositoryId: data.tasks[0].repositoryId,
    action: 'move',
    sessionMoves: [{ sessionId, fromTaskId: sourceTaskId, toTaskId: targetTaskId }],
    taskLifecycle: [],
    factImpacts: [
      { factId: 'fact-auth-boundary', fromTaskId: sourceTaskId, toTaskId: targetTaskId, mixedProvenance: false, sessionIds: [sessionId] },
      { factId: 'fact-tenant-isolation', fromTaskId: sourceTaskId, mixedProvenance: true, sessionIds: [sessionId, 'session-auth-3'] },
    ],
    requestFingerprint: `fingerprint-${operationId}`,
    occurredAt: '2025-05-21T00:01:00Z',
  };
}

function relationshipGraph(taskId: string): RelationshipGraphV1 {
  return {
    schemaVersion: 1,
    repositoryId: 'acme/api',
    nodes: [
      { id: taskId, kind: 'task', label: 'Refactor authentication boundary', taskId },
      { id: 'session-auth-2', kind: 'session', label: 'Authentication session', taskId },
      { id: 'src/middleware/auth.ts', kind: 'file', label: 'src/middleware/auth.ts', taskId },
    ],
    edges: [
      {
        id: 'edge-task-session',
        kind: 'task-has-session',
        from: taskId,
        to: 'session-auth-2',
        provenanceIds: ['event-task-session'],
        sourceKind: 'canonical_event',
        observedAt: '2025-05-21T00:00:00Z',
        verified: true,
      },
      {
        id: 'edge-session-file',
        kind: 'session-changed-file',
        from: 'session-auth-2',
        to: 'src/middleware/auth.ts',
        provenanceIds: ['checkpoint-2'],
        sourceKind: 'projection',
        observedAt: '2025-05-21T00:00:00Z',
        verified: true,
      },
    ],
  };
}

function factRefreshOperation(taskId: string, status: AiFactRefreshOperationV1['status']): AiFactRefreshOperationV1 {
  const candidates: AiFactCandidateV1[] = [
    {
      schemaVersion: 1,
      id: 'ai-candidate-update-auth',
      operationId: 'fact-refresh-operation-1',
      action: 'update',
      factId: 'fact-auth-boundary',
      kind: 'decision',
      content: 'Authentication handlers should continue to depend on AuthContext only.',
      reason: 'The verified pack shows a newer passing middleware test.',
      status: 'pending',
    },
    {
      schemaVersion: 1,
      id: 'ai-candidate-deprecate-tenant',
      operationId: 'fact-refresh-operation-1',
      action: 'deprecate',
      factId: 'fact-tenant-isolation',
      kind: 'constraint',
      content: 'Review whether the captured tenant constraint is stale after the verified rename.',
      reason: 'The verified file status changed after capture.',
      status: 'pending',
    },
  ];
  return {
    schemaVersion: 1,
    operationId: 'fact-refresh-operation-1',
    repositoryId: 'acme/api',
    taskId,
    status,
    requestFingerprint: 'fact-refresh-fingerprint-1',
    candidates: status === 'completed' ? candidates : [],
    createdAt: '2025-05-21T00:00:00Z',
    updatedAt: status === 'completed' ? '2025-05-21T00:01:00Z' : '2025-05-21T00:00:00Z',
  };
}

function agentLineage(taskId: string): AgentV1[] {
  return [
    {
      schemaVersion: 1,
      id: 'agent-primary',
      repositoryId: 'acme/api',
      threadId: 'thread-primary-01HZX4AUTH',
      sessionId: 'session-auth-2',
      taskId,
      sourceKind: 'interactive',
      role: 'primary',
      status: 'completed',
      name: 'Primary implementation',
      outputSummary: 'Implemented the verified authentication boundary.',
      files: ['src/middleware/auth.ts'],
      tests: ['auth middleware suite'],
      observedAt: '2025-05-21T00:00:00Z',
      associationState: 'linked',
    },
    {
      schemaVersion: 1,
      id: 'agent-ui',
      repositoryId: 'acme/api',
      threadId: 'thread-agent-ui-01HZX4AUTH',
      parentThreadId: 'thread-primary-01HZX4AUTH',
      taskId,
      sourceKind: 'subAgent',
      role: 'ui_verification',
      status: 'completed',
      name: 'UI verification',
      outputSummary: 'Verified responsive UI behavior.',
      files: ['ui/src/App.tsx'],
      tests: ['npm test'],
      observedAt: '2025-05-21T00:00:01Z',
      associationState: 'degraded',
      degradedReason: 'Some bounded output fields were unavailable.',
    },
    {
      schemaVersion: 1,
      id: 'agent-orphan',
      repositoryId: 'acme/api',
      threadId: 'thread-agent-orphan-01HZX4AUTH',
      parentThreadId: 'thread-missing-parent',
      taskId,
      sourceKind: 'subAgentReview',
      role: 'reviewer',
      status: 'unknown',
      name: 'Orphaned review observation',
      files: [],
      tests: [],
      observedAt: '2025-05-21T00:00:02Z',
      associationState: 'unlinked',
      degradedReason: 'Parent task was not returned by the local App Server.',
    },
  ];
}
