import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from './App';
import { CodexSyncStatus } from './components/CodexSyncStatus';
import { fallbackData } from './data/fallback';
import { I18nProvider } from './i18n';
import type { BootstrapData, CodexImportReportV1, RepositoryOverviewV1 } from './types';

const overview: RepositoryOverviewV1[] = [
  {
    repositoryId: 'repo-alpha',
    primaryRoot: '/work/alpha',
    taskCount: 1,
    recentActivityAt: '2026-07-22T01:00:00Z',
    recordStatus: 'ready',
  },
  {
    repositoryId: 'repo-beta',
    primaryRoot: '/work/beta',
    taskCount: 1,
    recentActivityAt: '2026-07-22T02:00:00Z',
    recordStatus: 'degraded',
  },
];

describe('Codex project synchronization', () => {
  beforeEach(() => {
    localStorage.clear();
    sessionStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('preserves preferences and isolates selection, automatic sync, manual sync, and the all-project view', async () => {
    localStorage.setItem('previously-on:preferences:v1', JSON.stringify({
      schemaVersion: 1,
      language: 'ko',
      repositoryId: 'repo-beta',
    }));
    const imports = new Map<string, number>();
    const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      if (path === '/api/overview') return response({ repositories: overview });
      if (path.startsWith('/api/bootstrap?')) {
        const repositoryId = new URL(path, 'http://previously.test').searchParams.get('repositoryId');
        return response(bootstrapFor(repositoryId!));
      }
      if (path === '/api/imports/codex') {
        const repositoryId = JSON.parse(String(init?.body)).repositoryId as string;
        imports.set(repositoryId, (imports.get(repositoryId) ?? 0) + 1);
        return response(reportFor(repositoryId));
      }
      throw new Error(`unexpected request ${path}`);
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();

    render(<App />);

    expect(await screen.findByRole('combobox', { name: '프로젝트 선택' })).toHaveValue('repo-beta');
    expect(await screen.findByRole('heading', { name: 'Beta task' })).toBeInTheDocument();
    expect(screen.getAllByText('진행 중').length).toBeGreaterThan(0);
    await waitFor(() => expect(imports.get('repo-beta')).toBe(1));
    expect(fetchMock.mock.calls.some(([input]) => String(input) === '/api/bootstrap?repositoryId=repo-beta')).toBe(true);
    expect(screen.queryByRole('heading', { name: 'Alpha task' })).not.toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem('previously-on:preferences:v1') ?? '{}')).toEqual({
      schemaVersion: 1,
      language: 'ko',
      repositoryId: 'repo-beta',
    });

    const selector = screen.getByRole('combobox', { name: '프로젝트 선택' });
    await user.selectOptions(selector, '__all__');
    expect(await screen.findByRole('heading', { name: '전체 프로젝트' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /alpha/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /beta/ })).toBeInTheDocument();
    expect(screen.getByText('성능 저하')).toBeInTheDocument();
    await Promise.resolve();
    expect(imports.get('repo-beta')).toBe(1);
    expect(imports.get('repo-alpha')).toBeUndefined();

    await user.selectOptions(screen.getByRole('combobox', { name: '프로젝트 선택' }), 'repo-alpha');
    expect(await screen.findByRole('heading', { name: 'Alpha task' })).toBeInTheDocument();
    await waitFor(() => expect(imports.get('repo-alpha')).toBe(1));
    expect(screen.queryByRole('heading', { name: 'Beta task' })).not.toBeInTheDocument();

    await user.selectOptions(screen.getByRole('combobox', { name: '프로젝트 선택' }), 'repo-beta');
    expect(await screen.findByRole('heading', { name: 'Beta task' })).toBeInTheDocument();
    await Promise.resolve();
    expect(imports.get('repo-beta')).toBe(1);

    await user.click(screen.getByRole('button', { name: 'Codex 앱 기록 동기화' }));
    await waitFor(() => expect(imports.get('repo-beta')).toBe(2));
    expect(JSON.parse(localStorage.getItem('previously-on:preferences:v1') ?? '{}')).toEqual({
      schemaVersion: 1,
      language: 'ko',
      repositoryId: 'repo-beta',
    });
  });

  it('ignores a late response from the previous repository without clearing the current pending state', async () => {
    localStorage.setItem('previously-on:preferences:v1', JSON.stringify({ schemaVersion: 1, language: 'en', repositoryId: 'repo-alpha' }));
    const alphaSync = deferred<CodexImportReportV1>();
    const betaSync = deferred<CodexImportReportV1>();
    installDeferredSyncFetch(alphaSync, betaSync);
    const user = userEvent.setup();

    render(<App />);

    expect(await screen.findByRole('heading', { name: 'Alpha task' })).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Synchronizing…' })).toBeDisabled();
    await user.selectOptions(screen.getByRole('combobox', { name: 'Project selector' }), 'repo-beta');
    expect(await screen.findByRole('heading', { name: 'Beta task' })).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Synchronizing…' })).toBeDisabled();

    alphaSync.resolve(reportFor('repo-alpha', 'unsupported'));
    await Promise.resolve();
    expect(screen.getByRole('button', { name: 'Synchronizing…' })).toBeDisabled();
    expect(screen.queryByText('App Server unsupported')).not.toBeInTheDocument();

    betaSync.resolve(reportFor('repo-beta'));
    expect(await screen.findByText('Synchronization complete')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Sync Codex app history' })).toBeEnabled());
  });

  it('ignores a late error from the previous repository without overwriting current error or pending state', async () => {
    localStorage.setItem('previously-on:preferences:v1', JSON.stringify({ schemaVersion: 1, language: 'en', repositoryId: 'repo-alpha' }));
    const alphaSync = deferred<CodexImportReportV1>();
    const betaSync = deferred<CodexImportReportV1>();
    installDeferredSyncFetch(alphaSync, betaSync);
    const user = userEvent.setup();

    render(<App />);

    expect(await screen.findByRole('heading', { name: 'Alpha task' })).toBeInTheDocument();
    await user.selectOptions(screen.getByRole('combobox', { name: 'Project selector' }), 'repo-beta');
    expect(await screen.findByRole('heading', { name: 'Beta task' })).toBeInTheDocument();

    alphaSync.reject(new Error('alpha sync failed late'));
    await Promise.resolve();
    expect(screen.getByRole('button', { name: 'Synchronizing…' })).toBeDisabled();
    expect(screen.queryByText('alpha sync failed late')).not.toBeInTheDocument();

    betaSync.resolve(reportFor('repo-beta'));
    expect(await screen.findByText('Synchronization complete')).toBeInTheDocument();
    expect(screen.queryByText('alpha sync failed late')).not.toBeInTheDocument();
  });

  it.each([
    ['complete', '동기화 완료', 'Codex Desktop 기록을 이 로컬 프로젝트로 가져왔습니다.'],
    ['degraded', '동기화 성능 저하', '일부 로컬 Codex Desktop 기록을 가져오지 못했습니다. 항목 수와 기술 세부 정보를 확인하세요.'],
    ['unsupported', 'App Server 미지원', '이 로컬 Codex App Server는 Codex Desktop 기록 가져오기를 지원하지 않습니다.'],
  ] as const)('translates the %s sync reason while isolating raw details', (status, title, description) => {
    localStorage.setItem('previously-on:preferences:v1', JSON.stringify({ schemaVersion: 1, language: 'ko' }));
    render(<I18nProvider><CodexSyncStatus report={reportFor('repo-alpha', status)} /></I18nProvider>);

    expect(screen.getByText(title)).toBeInTheDocument();
    expect(screen.getByText(description)).toBeInTheDocument();
    if (status !== 'complete') {
      const details = screen.getByText('기술 세부 정보').closest('details');
      expect(details).toContainElement(screen.getByText('unsupported test detail'));
    }
  });

  it('renders the registered-empty Codex Desktop flow in Korean', async () => {
    localStorage.setItem('previously-on:preferences:v1', JSON.stringify({ schemaVersion: 1, language: 'ko', repositoryId: 'repo-alpha' }));
    const empty = bootstrapFor('repo-alpha');
    empty.repository.state = 'registered-empty';
    empty.tasks = [];
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const requestPath = String(input);
      if (requestPath === '/api/overview') return response({ repositories: [overview[0]] });
      if (requestPath.startsWith('/api/bootstrap?')) return response(empty);
      if (requestPath === '/api/imports/codex') return response(reportFor('repo-alpha'));
      throw new Error(`unexpected request ${requestPath}`);
    }));

    render(<App />);

    expect((await screen.findAllByText('등록됨 · 첫 체크포인트 대기 중')).length).toBeGreaterThan(0);
    expect(screen.getByText('Codex Desktop에서 작업하기')).toBeInTheDocument();
    expect(screen.getByText('이 기기에서 가져오기')).toBeInTheDocument();
    expect(screen.queryByText('Start a captured Codex session')).not.toBeInTheDocument();
  });

  it('translates an API error code and keeps the raw failure under technical details', async () => {
    localStorage.setItem('previously-on:preferences:v1', JSON.stringify({ schemaVersion: 1, language: 'ko', repositoryId: 'repo-alpha' }));
    vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
      const requestPath = String(input);
      if (requestPath === '/api/overview') return response({ repositories: [overview[0]] });
      if (requestPath.startsWith('/api/bootstrap?')) return response(bootstrapFor('repo-alpha'));
      if (requestPath === '/api/imports/codex') {
        return {
          ok: false,
          status: 409,
          json: async () => ({ errorCode: 'conflict', technicalDetails: ['socket failure raw detail'] }),
        };
      }
      throw new Error(`unexpected request ${requestPath}`);
    }));

    render(<App />);

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('요청을 완료하기 전에 로컬 데이터가 변경되었습니다. 새로고침한 뒤 다시 시도하세요.');
    const details = within(alert).getByText('기술 세부 정보').closest('details');
    expect(details).toContainElement(within(alert).getByText('socket failure raw detail'));
  });
});

function installDeferredSyncFetch(
  alphaSync: ReturnType<typeof deferred<CodexImportReportV1>>,
  betaSync: ReturnType<typeof deferred<CodexImportReportV1>>,
) {
  vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input);
    if (path === '/api/overview') return response({ repositories: overview });
    if (path.startsWith('/api/bootstrap?')) {
      const repositoryId = new URL(path, 'http://previously.test').searchParams.get('repositoryId');
      return response(bootstrapFor(repositoryId!));
    }
    if (path === '/api/imports/codex') {
      const repositoryId = JSON.parse(String(init?.body)).repositoryId as string;
      const report = await (repositoryId === 'repo-alpha' ? alphaSync.promise : betaSync.promise);
      return response(report);
    }
    throw new Error(`unexpected request ${path}`);
  }));
}

function bootstrapFor(repositoryId: string): BootstrapData {
  const data = structuredClone(fallbackData);
  const alpha = repositoryId === 'repo-alpha';
  data.repository = {
    ...data.repository,
    name: repositoryId,
    path: alpha ? '/work/alpha' : '/work/beta',
  };
  data.tasks = [{
    ...data.tasks[0],
    id: `${repositoryId}-task`,
    repositoryId,
    title: alpha ? 'Alpha task' : 'Beta task',
    checkpointIds: [],
    codebase: {
      ...data.tasks[0].codebase,
      repositoryName: repositoryId,
      registeredRoot: data.repository.path,
      worktreeRoot: data.repository.path,
    },
  }];
  data.checkpoints = [];
  data.facts = [];
  data.evidence = [];
  data.sessions = [];
  data.contextPacks = {};
  data.resumeCandidate = undefined;
  return data;
}

function reportFor(repositoryId: string, status: CodexImportReportV1['status'] = 'complete'): CodexImportReportV1 {
  return {
    schemaVersion: 1,
    repositoryId,
    status,
    reasonCode: status === 'complete' ? 'synchronized' : status === 'unsupported' ? 'app_server_unsupported' : 'partial_import',
    importedTaskCount: status === 'complete' ? 1 : 0,
    semanticEventCount: status === 'complete' ? 1 : 0,
    duplicateCount: 0,
    missingOrUnknownItems: status === 'complete' ? [] : ['thread/list unavailable'],
    lastSyncedAt: '2026-07-22T03:00:00Z',
    capability: {
      status,
      testedCodexVersion: '0.1.0',
      warnings: [],
    },
    coverage: { status: status === 'complete' ? 'complete' : 'degraded', captured: [], missing: [], warnings: [] },
    semanticCoverage: { status: status === 'complete' ? 'complete' : 'degraded', captured: [], missing: [], warnings: [] },
    notices: [],
    observedAgentCount: 0,
    technicalDetails: status === 'complete' ? [] : ['unsupported test detail'],
  };
}

function response<T>(payload: T) {
  return { ok: true, status: 200, json: async () => payload };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}
