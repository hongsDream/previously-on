import { render, screen, waitFor, within } from '@testing-library/react';
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
    expect(screen.getByRole('status')).toHaveTextContent('Local API unavailable');
    expect(screen.getAllByText('Authentication boundary will be enforced in middleware layer; handlers will depend on AuthContext interface only.').length).toBeGreaterThan(0);
    expect(screen.getAllByText('864 tokens').length).toBeGreaterThan(0);
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
    expect(screen.getByText(/ready to capture the next Codex session/)).toBeInTheDocument();
    expect(screen.queryByText(/sample workspace/)).not.toBeInTheDocument();
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
