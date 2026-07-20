import { describe, expect, it } from 'vitest';
import { fallbackData } from '../data/fallback';
import type { BootstrapData } from '../types';
import {
  normalizeBootstrap,
  resolveTaskSelection,
  selectWorkspace,
} from './workspace';

describe('workspace selection', () => {
  it('keeps the established default checkpoint and evidence selection', () => {
    expect(resolveTaskSelection(fallbackData)).toEqual({
      taskId: 'task_01HZX3P7K2BBQW9F7D8Z1A2C3V',
      checkpointId: 'checkpoint-2',
      evidenceId: 'ev_01HZX4C9Y7T2R6D8F3G1K8LMN',
    });
  });

  it('derives only task-scoped checkpoint, evidence, fact, and contract state', () => {
    const selection = resolveTaskSelection(
      fallbackData,
      'task_01HZX3P7K2BBQW9F7D8Z1A2C3V',
      'checkpoint-2',
      'ev_01HZX4C9Y7T2R6D8F3G1K8LMO',
    );
    const workspace = selectWorkspace(fallbackData, selection, fallbackData.tasks);

    expect(workspace?.selectedTask.id).toBe(selection.taskId);
    expect(workspace?.selectedCheckpoint?.id).toBe(selection.checkpointId);
    expect(workspace?.selectedEvidence?.id).toBe(selection.evidenceId);
    expect(workspace?.selectedFact?.id).toBe('fact-tenant-isolation');
    expect(workspace?.selectedContractEvaluation?.taskId).toBe(selection.taskId);
    expect(workspace?.evidenceAvailable).toBe(true);
  });

  it('normalizes missing additive bootstrap fields without inventing active state', () => {
    const legacyBootstrap = {
      ...fallbackData,
      repository: {
        ...fallbackData.repository,
        connected: false,
        state: undefined,
      },
      contracts: undefined,
      contractCandidates: undefined,
      contractEvaluations: undefined,
      factRefreshOperations: undefined,
      agents: undefined,
    } as unknown as BootstrapData;
    const normalized = normalizeBootstrap(legacyBootstrap);

    expect(normalized.repository.state).toBe('unregistered');
    expect(normalized.contracts).toEqual([]);
    expect(normalized.contractCandidates).toEqual([]);
    expect(normalized.contractEvaluations).toEqual([]);
    expect(normalized.factRefreshOperations).toEqual([]);
    expect(normalized.agents).toEqual([]);
  });
});
