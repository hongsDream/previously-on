import { useCallback } from 'react';
import type { Dispatch, SetStateAction } from 'react';
import {
  exportRepository,
  fetchBootstrap,
  fetchFactRefresh,
  purgeRepository,
  reviewFactRefreshCandidate,
  startFactRefresh,
  toUiError,
} from '../lib/api';
import type { FactCandidateReviewResponse, UiError } from '../lib/api';
import { useI18n } from '../i18n-context';
import { emptyWorkspaceSelection, type WorkspaceSelectionIds } from '../lib/workspace';
import type { AiFactRefreshOperationV1, BootstrapData, Fact, FactKind, Task } from '../types';
import type { PerformMutation } from './useMutationRunner';

interface RefreshActionsOptions {
  repositoryId: string | null;
  data: BootstrapData | null;
  selectedTask?: Task;
  selection: WorkspaceSelectionIds;
  offlineFallback: boolean;
  isUnregistered: boolean;
  mutationPending: boolean;
  setMutationPending: Dispatch<SetStateAction<boolean>>;
  setActionError: Dispatch<SetStateAction<UiError | null>>;
  setData: Dispatch<SetStateAction<BootstrapData | null>>;
  setSelection: Dispatch<SetStateAction<WorkspaceSelectionIds>>;
  installBootstrap: (next: BootstrapData, preferred?: Partial<WorkspaceSelectionIds>) => void;
  performMutation: PerformMutation;
}

export function useRefreshActions({
  repositoryId,
  data,
  selectedTask,
  selection,
  offlineFallback,
  isUnregistered,
  mutationPending,
  setMutationPending,
  setActionError,
  setData,
  setSelection,
  installBootstrap,
  performMutation,
}: RefreshActionsOptions) {
  const { t } = useI18n();
  const refreshBootstrap = useCallback(async () => {
    if (offlineFallback || mutationPending) return;
    setMutationPending(true);
    setActionError(null);
    try {
      installBootstrap(await fetchBootstrap(repositoryId ?? undefined), selection);
    } catch (error) {
      setActionError(toUiError(error, 'The local status could not be refreshed.'));
    } finally {
      setMutationPending(false);
    }
  }, [installBootstrap, mutationPending, offlineFallback, repositoryId, selection, setActionError, setMutationPending]);

  const exportData = useCallback(async () => {
    if (!data || offlineFallback || isUnregistered || mutationPending) return;
    setActionError(null);
    try {
      const exported = await exportRepository();
      const blob = new Blob([`${JSON.stringify(exported, null, 2)}\n`], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      const safeName = data.repository.name.replace(/[^a-zA-Z0-9._-]+/g, '-');
      link.href = url;
      link.download = `${safeName || 'previously-on'}-export.json`;
      link.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      setActionError(toUiError(error, 'The export could not be created.'));
    }
  }, [data, isUnregistered, mutationPending, offlineFallback, setActionError]);

  const purge = useCallback(async () => {
    if (!data || offlineFallback || isUnregistered || mutationPending) return;
    const confirmed = window.confirm(t('Permanently delete all PreviouslyOn data for {path}? This cannot be undone.', { path: data.repository.path }));
    if (!confirmed) return;
    const purged = await performMutation(purgeRepository);
    if (purged !== null) {
      setData((current) => current ? {
        ...current,
        repository: { ...current.repository, state: 'registered-empty', captureHealth: 'good' },
        tasks: [],
        checkpoints: [],
        facts: [],
        evidence: [],
        sessions: [],
        taskGroupingOperations: [],
        graphSummary: { nodeCount: 0, edgeCount: 0, verifiedEdgeCount: 0 },
        contractCandidates: [],
        contractEvaluation: null,
        resumeCandidate: undefined,
        contextPacks: {},
      } : current);
      setSelection(emptyWorkspaceSelection);
    }
  }, [data, isUnregistered, mutationPending, offlineFallback, performMutation, setData, setSelection, t]);

  const installFactRefreshOperation = useCallback((operation: AiFactRefreshOperationV1) => {
    setData((current) => current ? {
      ...current,
      factRefreshOperations: current.factRefreshOperations.some((item) => item.operationId === operation.operationId)
        ? current.factRefreshOperations.map((item) => item.operationId === operation.operationId ? operation : item)
        : [...current.factRefreshOperations, operation],
    } : current);
  }, [setData]);

  const start = useCallback(async (requestId: string): Promise<AiFactRefreshOperationV1 | null> => {
    if (!data || !selectedTask || offlineFallback || mutationPending || data.aiRefreshCapability.status !== 'ready') return null;
    const operation = await performMutation(() => startFactRefresh(selectedTask.id, requestId));
    if (operation) installFactRefreshOperation(operation);
    return operation;
  }, [data, installFactRefreshOperation, mutationPending, offlineFallback, performMutation, selectedTask]);

  const poll = useCallback(async (
    operationId: string,
    signal: AbortSignal,
  ): Promise<AiFactRefreshOperationV1 | null> => {
    if (offlineFallback) return null;
    try {
      const operation = await fetchFactRefresh(operationId, signal);
      installFactRefreshOperation(operation);
      return operation;
    } catch (error) {
      if (error instanceof DOMException && error.name === 'AbortError') return null;
      console.error('PreviouslyOn fact refresh polling failed', error);
      setActionError(toUiError(error, 'The local refresh status could not be checked.'));
      return null;
    }
  }, [installFactRefreshOperation, offlineFallback, setActionError]);

  const review = useCallback(async (
    operationId: string,
    candidateId: string,
    decision: 'accept' | 'reject',
    content?: string,
    kind?: FactKind,
  ) => {
    const result = await performMutation(() => reviewFactRefreshCandidate(operationId, candidateId, decision, content, kind));
    if (!result) return null;
    const reviewedFact = normalizeReviewedFact(result.fact);
    setData((current) => current ? {
      ...current,
      facts: reviewedFact
        ? [...current.facts.filter((fact) => fact.id !== reviewedFact.id), reviewedFact]
        : current.facts,
      factRefreshOperations: current.factRefreshOperations.map((operation) => operation.operationId === operationId ? {
        ...operation,
        candidates: operation.candidates.map((candidate) => candidate.id === candidateId ? result.candidate : candidate),
      } : operation),
    } : current);
    return result;
  }, [performMutation, setData]);

  return { refreshBootstrap, exportData, purge, start, poll, review };
}

function normalizeReviewedFact(fact: FactCandidateReviewResponse['fact']): Fact | null {
  if (!fact) return null;
  if ('text' in fact) return fact;
  return {
    id: fact.id,
    taskId: fact.taskId,
    kind: fact.kind,
    text: fact.content,
    status: fact.lifecycle,
    updatedAt: fact.updatedAt,
    evidenceIds: fact.evidenceIds ?? [],
    relatedFiles: [],
    mixedProvenance: false,
    provenanceSessionIds: [],
  };
}
