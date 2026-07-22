import type {
  AiFactCandidateV1,
  AiFactRefreshOperationV1,
  BootstrapData,
  ContractEvaluationV1,
  CodexImportReportV1,
  FactStatus,
  Fact,
  FactKind,
  RegressionCandidateDraftV1,
  RegressionCandidateV1,
  RegressionContractV1,
  RelationshipGraphV1,
  RepositoryOverviewV1,
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

export interface DoctorCheck {
  name: string;
  ok: boolean;
  detail: string;
}

export interface SetupCodexResponse {
  ok: true;
  repositoryPath: string;
  restartRequired: boolean;
  doctor: {
    healthy: boolean;
    checks: DoctorCheck[];
  };
}

export type ApiErrorCode = 'invalid_request' | 'forbidden' | 'not_found' | 'conflict' | 'internal_error';

export interface UiError {
  messageKey: string;
  technicalDetails: string[];
}

const API_ERROR_MESSAGES: Record<ApiErrorCode, string> = {
  invalid_request: 'The request could not be completed because its input was invalid.',
  forbidden: 'The local UI is not authorized to perform this request.',
  not_found: 'The requested local item could not be found.',
  conflict: 'Local data changed before this request could be completed. Refresh and try again.',
  internal_error: 'PreviouslyOn could not complete the local request.',
};

export class ApiResponseError extends Error {
  constructor(
    readonly errorCode: ApiErrorCode,
    readonly technicalDetails: string[],
  ) {
    super(API_ERROR_MESSAGES[errorCode]);
    this.name = 'ApiResponseError';
  }
}

export class ApiUnavailableError extends Error {
  constructor(message = 'PreviouslyOn API is unavailable') {
    super(message);
    this.name = 'ApiUnavailableError';
  }
}

export function toUiError(error: unknown, fallbackMessageKey: string): UiError {
  if (error instanceof ApiResponseError) {
    return {
      messageKey: API_ERROR_MESSAGES[error.errorCode],
      technicalDetails: error.technicalDetails,
    };
  }
  if (error instanceof ApiUnavailableError) {
    return { messageKey: 'PreviouslyOn API is unavailable', technicalDetails: [] };
  }
  return { messageKey: fallbackMessageKey, technicalDetails: [] };
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
    const payload = await response.json().catch(() => null) as {
      errorCode?: unknown;
      technicalDetails?: unknown;
      error?: unknown;
    } | null;
    const errorCode = isApiErrorCode(payload?.errorCode)
      ? payload.errorCode
      : errorCodeForStatus(response.status);
    const technicalDetails = Array.isArray(payload?.technicalDetails)
      ? payload.technicalDetails.filter((detail): detail is string => typeof detail === 'string' && detail.trim().length > 0)
      : typeof payload?.error === 'string' && payload.error.trim()
        ? [payload.error]
        : [];
    throw new ApiResponseError(errorCode, technicalDetails);
  }
  return response.json() as Promise<T>;
}

function isApiErrorCode(value: unknown): value is ApiErrorCode {
  return typeof value === 'string' && value in API_ERROR_MESSAGES;
}

function errorCodeForStatus(status: number): ApiErrorCode {
  if (status === 400) return 'invalid_request';
  if (status === 403) return 'forbidden';
  if (status === 404) return 'not_found';
  if (status === 409) return 'conflict';
  return 'internal_error';
}

export function fetchBootstrap(repositoryId?: string, signal?: AbortSignal) {
  const query = repositoryId ? `?repositoryId=${encodeURIComponent(repositoryId)}` : '';
  return request<BootstrapData>(`/api/bootstrap${query}`, { signal });
}

export function fetchRepositoryOverview(signal?: AbortSignal) {
  return request<{ repositories: RepositoryOverviewV1[] }>('/api/overview', { signal });
}

export function syncCodexRepository(repositoryId: string) {
  return request<CodexImportReportV1>('/api/imports/codex', {
    method: 'POST',
    body: JSON.stringify({ repositoryId }),
  });
}

export function setupCodex(repositoryPath: string) {
  return request<SetupCodexResponse>('/api/setup/codex', {
    method: 'POST',
    body: JSON.stringify({ repositoryPath, confirmed: true }),
  });
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
