import type {
  AiFactCandidateV1,
  AiFactRefreshOperationV1,
  BootstrapData,
  ContractEvaluationV1,
  FactStatus,
  Fact,
  FactKind,
  RegressionCandidateDraftV1,
  RegressionCandidateV1,
  RegressionContractV1,
  RelationshipGraphV1,
  TaskGroupingOperationV1,
  TaskGroupingPreviewV1,
  TaskGroupingRequestV1,
  TaskUpdateV1,
} from '../types';

export interface FactCandidateReviewResponse {
  ok: true;
  candidate: AiFactCandidateV1;
  fact?: Fact | {
    id: string;
    taskId: string;
    kind: FactKind;
    lifecycle: FactStatus;
    content: string;
    updatedAt: string;
    evidenceIds?: string[];
  } | null;
}

export interface ContractMutationResponse {
  ok?: true;
  candidate?: RegressionCandidateV1;
  contract?: RegressionContractV1;
  contracts?: RegressionContractV1[];
  contractCandidates?: RegressionCandidateV1[];
  contractEvaluation?: ContractEvaluationV1 | null;
}

export class ApiUnavailableError extends Error {
  constructor(message = 'PreviouslyOn API is unavailable') {
    super(message);
    this.name = 'ApiUnavailableError';
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      ...init,
      headers: {
        Accept: 'application/json',
        ...(init?.body ? { 'Content-Type': 'application/json' } : {}),
        ...init?.headers,
      },
    });
  } catch (error) {
    if (error instanceof DOMException && error.name === 'AbortError') throw error;
    throw new ApiUnavailableError();
  }

  if (!response.ok) {
    if (import.meta.env.DEV && response.status >= 500) {
      throw new ApiUnavailableError();
    }
    const payload = await response.json().catch(() => null) as { error?: unknown } | null;
    const message = typeof payload?.error === 'string' && payload.error.trim()
      ? payload.error
      : `API request failed (${response.status})`;
    throw new Error(message);
  }
  return response.json() as Promise<T>;
}

export function fetchBootstrap(signal?: AbortSignal) {
  return request<BootstrapData>('/api/bootstrap', { signal });
}

export function updateFactStatus(id: string, status: FactStatus, supersedesFactId?: string) {
  return request<{ ok: true }>(`/api/facts/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify({ status, supersedesFactId }),
  });
}

export function updateFact(id: string, status: FactStatus, content: string, deprecatedAfterCommit: string) {
  return request<{ ok: true; text: string; status: FactStatus; updatedAt: string; deprecatedAfterCommit?: string }>(`/api/facts/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify({ status, content, deprecatedAfterCommit }),
  });
}

export function updateSession(id: string, excluded: boolean) {
  return request<{ ok: true; sessionId: string; excluded: boolean }>(`/api/sessions/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify({ excluded }),
  });
}

export function revalidateFact(id: string) {
  return request<{ ok: true; freshness: 'fresh' | 'stale' | 'broken'; validatedAt: string }>(`/api/facts/${encodeURIComponent(id)}/revalidate`, {
    method: 'POST',
  });
}

export function exportRepository() {
  return request<Record<string, unknown>>('/api/export');
}

export function purgeRepository() {
  return request<{ ok: true; repositoryId: string }>('/api/repository', {
    method: 'DELETE',
  });
}

export function updateTask(id: string, update: TaskUpdateV1) {
  return request<{ ok: true }>(`/api/tasks/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify(update),
  });
}

export function previewTaskGrouping(requestBody: TaskGroupingRequestV1) {
  return request<TaskGroupingPreviewV1>('/api/task-grouping/preview', {
    method: 'POST',
    body: JSON.stringify(requestBody),
  });
}

export function applyTaskGrouping(requestBody: TaskGroupingRequestV1) {
  return request<{ ok: true; operation: TaskGroupingOperationV1 }>('/api/task-grouping', {
    method: 'POST',
    body: JSON.stringify(requestBody),
  });
}

export function undoTaskGrouping(operationId: string) {
  return request<{ ok: true; operation: TaskGroupingOperationV1 }>(`/api/task-grouping/${encodeURIComponent(operationId)}/undo`, {
    method: 'POST',
  });
}

export function fetchRelationshipGraph(repositoryId: string, taskId?: string, signal?: AbortSignal) {
  const query = new URLSearchParams({ repository: repositoryId });
  if (taskId) query.set('task', taskId);
  return request<RelationshipGraphV1>(`/api/graph?${query.toString()}`, { signal });
}

export function startFactRefresh(taskId: string, requestId: string) {
  return request<AiFactRefreshOperationV1>(`/api/tasks/${encodeURIComponent(taskId)}/fact-refresh`, {
    method: 'POST',
    body: JSON.stringify({ requestId }),
  });
}

export function fetchFactRefresh(operationId: string, signal?: AbortSignal) {
  return request<AiFactRefreshOperationV1>(`/api/fact-refresh/${encodeURIComponent(operationId)}`, { signal });
}

export function reviewFactRefreshCandidate(
  operationId: string,
  candidateId: string,
  decision: 'accept' | 'reject',
  content?: string,
  kind?: FactKind,
) {
  return request<FactCandidateReviewResponse>(`/api/fact-refresh/${encodeURIComponent(operationId)}/candidates/${encodeURIComponent(candidateId)}`, {
    method: 'PATCH',
    body: JSON.stringify({
      decision,
      ...(content === undefined ? {} : { content }),
      ...(kind === undefined ? {} : { kind }),
    }),
  });
}

export function createContractCandidate(candidate: RegressionCandidateDraftV1) {
  return request<ContractMutationResponse>('/api/contract-candidates', {
    method: 'POST',
    body: JSON.stringify(candidate),
  });
}

export function updateContractCandidate(id: string, candidate: RegressionCandidateDraftV1) {
  return request<ContractMutationResponse>(`/api/contract-candidates/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify(candidate),
  });
}

export function approveContractCandidate(id: string) {
  return request<ContractMutationResponse>(`/api/contract-candidates/${encodeURIComponent(id)}/approve`, {
    method: 'POST',
  });
}

export function supersedeRegressionContract(id: string, supersededBy: string) {
  return request<ContractMutationResponse>(`/api/contracts/${encodeURIComponent(id)}/supersede`, {
    method: 'POST',
    body: JSON.stringify({ supersededBy }),
  });
}
