import { useCallback } from 'react';
import type { Dispatch, SetStateAction } from 'react';
import {
  applyTaskGrouping,
  fetchBootstrap,
  previewTaskGrouping,
  undoTaskGrouping,
  updateTask,
} from '../lib/api';
import type { WorkspaceSelectionIds } from '../lib/workspace';
import type {
  BootstrapData,
  Task,
  TaskGroupingPreviewV1,
  TaskGroupingRequestV1,
  TaskUpdateV1,
} from '../types';
import type { PerformMutation } from './useMutationRunner';

interface TaskActionsOptions {
  repositoryId: string | null;
  selectedTask?: Task;
  selection: WorkspaceSelectionIds;
  offlineFallback: boolean;
  mutationPending: boolean;
  installBootstrap: (next: BootstrapData, preferred?: Partial<WorkspaceSelectionIds>) => void;
  setGraphRefreshVersion: Dispatch<SetStateAction<number>>;
  performMutation: PerformMutation;
}

export function useTaskActions({
  repositoryId,
  selectedTask,
  selection,
  offlineFallback,
  mutationPending,
  installBootstrap,
  setGraphRefreshVersion,
  performMutation,
}: TaskActionsOptions) {
  const installRefreshedBootstrap = useCallback((next: BootstrapData, preferredTaskId?: string) => {
    installBootstrap(next, {
      taskId: preferredTaskId ?? selectedTask?.id,
      checkpointId: selection.checkpointId,
      evidenceId: selection.evidenceId,
    });
    setGraphRefreshVersion((version) => version + 1);
  }, [installBootstrap, selectedTask, selection, setGraphRefreshVersion]);

  const mutateAndRefresh = useCallback(async (
    mutation: () => Promise<unknown>,
    preferredTaskId?: string,
  ) => {
    if (offlineFallback || mutationPending || !selectedTask) return false;
    const refreshed = await performMutation(async () => {
      await mutation();
      return fetchBootstrap(repositoryId ?? undefined);
    });
    if (!refreshed) return false;
    installRefreshedBootstrap(refreshed, preferredTaskId ?? selectedTask.id);
    return true;
  }, [installRefreshedBootstrap, mutationPending, offlineFallback, performMutation, repositoryId, selectedTask]);

  const update = useCallback((taskUpdate: TaskUpdateV1) => {
    if (!selectedTask) return Promise.resolve(false);
    return mutateAndRefresh(() => updateTask(selectedTask.id, taskUpdate), selectedTask.id);
  }, [mutateAndRefresh, selectedTask]);

  const previewGrouping = useCallback(async (
    request: TaskGroupingRequestV1,
  ): Promise<TaskGroupingPreviewV1 | null> => {
    if (offlineFallback || mutationPending) return null;
    return performMutation(() => previewTaskGrouping(request));
  }, [mutationPending, offlineFallback, performMutation]);

  const applyGrouping = useCallback((request: TaskGroupingRequestV1) => (
    mutateAndRefresh(() => applyTaskGrouping(request), request.fromTaskId)
  ), [mutateAndRefresh]);

  const undoGrouping = useCallback((operationId: string) => {
    if (!selectedTask) return Promise.resolve(false);
    return mutateAndRefresh(() => undoTaskGrouping(operationId), selectedTask.id);
  }, [mutateAndRefresh, selectedTask]);

  return { update, previewGrouping, applyGrouping, undoGrouping };
}
