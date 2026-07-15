import type {
  BootstrapData,
  ContractEvaluationV1,
  FactStatus,
  RegressionCandidateDraftV1,
  RegressionCandidateV1,
  RegressionContractV1,
  TaskStatus,
} from '../types';

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
  } catch {
    throw new ApiUnavailableError();
  }

  if (!response.ok) {
    if (import.meta.env.DEV && response.status >= 500) {
      throw new ApiUnavailableError();
    }
    throw new Error(`API request failed (${response.status})`);
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

export function updateTaskStatus(id: string, status: TaskStatus) {
  return request<{ ok: true; status: TaskStatus; updatedAt: string }>(`/api/tasks/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify({ status }),
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
