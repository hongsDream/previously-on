import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { fallbackData } from './data/fallback';

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
    expect(screen.getByText('Code map')).toBeInTheDocument();

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
    expect(taskButtons).toHaveLength(2);
    expect(sessionButtons).toHaveLength(2);
    expect(sessionButtons[1]).toBeEnabled();

    await user.click(sessionButtons[1]);
    expect(screen.getByRole('main', { name: 'Project overview' })).toBeInTheDocument();
    expect(sessionButtons[1]).toHaveClass('active');

    await user.click(taskButtons[1]);
    expect(taskButtons[1]).toHaveClass('active');
    expect(screen.getByText('Code map')).toBeInTheDocument();
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
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        repository: { name: 'new-repo', path: '/tmp/new-repo', branch: 'main', connected: true, captureHealth: 'good' },
        tasks: [],
        checkpoints: [],
        facts: [],
        evidence: [],
        contextPacks: {},
      }),
    }));

    render(<App />);

    expect(await screen.findByRole('heading', { name: 'new-repo is connected' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Regression contracts' })).toBeInTheDocument();
    expect(screen.getByText('Readiness unavailable')).toBeInTheDocument();
    expect(screen.getByText(/ready to capture the next Codex session/)).toBeInTheDocument();
    expect(screen.queryByText(/sample workspace/)).not.toBeInTheDocument();
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
