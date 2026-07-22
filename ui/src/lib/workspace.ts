import type { BootstrapData, Checkpoint } from '../types';

export interface WorkspaceSelectionIds {
  taskId: string;
  checkpointId: string;
  evidenceId: string;
}

export interface WorkspaceSelection {
  selectedTask: BootstrapData['tasks'][number];
  taskCheckpoints: Checkpoint[];
  selectedCheckpoint?: Checkpoint;
  selectedEvidence?: BootstrapData['evidence'][number];
  taskEvidence: BootstrapData['evidence'];
  selectedFact?: BootstrapData['facts'][number];
  evidenceAvailable: boolean;
  resumeCandidate?: BootstrapData['resumeCandidate'];
  selectedContractEvaluation: BootstrapData['contractEvaluation'];
}

export const emptyWorkspaceSelection: WorkspaceSelectionIds = {
  taskId: '',
  checkpointId: '',
  evidenceId: '',
};

export function normalizeBootstrap(bootstrap: BootstrapData): BootstrapData {
  const repositoryState = bootstrap.repository.state ?? inferRepositoryState(bootstrap);
  return {
    ...bootstrap,
    repository: { ...bootstrap.repository, state: repositoryState },
    contracts: bootstrap.contracts ?? [],
    contractCandidates: bootstrap.contractCandidates ?? [],
    contractEvaluation: bootstrap.contractEvaluation ?? null,
    contractEvaluations: bootstrap.contractEvaluations ?? [],
    taskGroupingOperations: bootstrap.taskGroupingOperations ?? [],
    graphSummary: bootstrap.graphSummary ?? { nodeCount: 0, edgeCount: 0, verifiedEdgeCount: 0 },
    aiRefreshCapability: bootstrap.aiRefreshCapability ?? {
      status: 'blocked',
      profileName: 'previously-input-only',
      reasonCode: 'verification_blocked',
      technicalDetails: ['The local API did not provide a verified AI refresh capability.'],
    },
    factRefreshOperations: bootstrap.factRefreshOperations ?? [],
    agents: bootstrap.agents ?? [],
    sessions: bootstrap.sessions ?? [],
    facts: (bootstrap.facts ?? []).map((fact) => ({
      ...fact,
      taskId: fact.taskId ?? bootstrap.tasks[0]?.id ?? '',
      kind: fact.kind ?? 'note',
      relatedFiles: fact.relatedFiles ?? [],
      mixedProvenance: fact.mixedProvenance ?? false,
      provenanceSessionIds: fact.provenanceSessionIds ?? [],
    })),
    evidence: (bootstrap.evidence ?? []).map((evidence) => ({
      ...evidence,
      sessionId: evidence.sessionId ?? '',
      excludedSession: evidence.excludedSession ?? false,
    })),
  };
}

export function resolveTaskSelection(
  data: BootstrapData,
  preferredTaskId?: string,
  preferredCheckpointId?: string,
  preferredEvidenceId?: string,
): WorkspaceSelectionIds {
  const task = data.tasks.find((candidate) => candidate.id === preferredTaskId) ?? data.tasks[0];
  if (!task) return emptyWorkspaceSelection;
  const checkpointId = preferredCheckpointId && task.checkpointIds.includes(preferredCheckpointId)
    ? preferredCheckpointId
    : task.checkpointIds[1] ?? task.checkpointIds[0] ?? '';
  const taskCheckpointIds = new Set(task.checkpointIds);
  const preferredEvidence = data.evidence.find((evidence) => evidence.id === preferredEvidenceId);
  const evidence = preferredEvidence && taskCheckpointIds.has(preferredEvidence.checkpointId)
    ? preferredEvidence
    : data.evidence.find((candidate) => candidate.checkpointId === checkpointId)
      ?? data.evidence.find((candidate) => taskCheckpointIds.has(candidate.checkpointId));
  return { taskId: task.id, checkpointId, evidenceId: evidence?.id ?? '' };
}

export function selectWorkspace(
  data: BootstrapData,
  selection: WorkspaceSelectionIds,
  filteredTasks: BootstrapData['tasks'],
): WorkspaceSelection | null {
  const selectedTask = data.tasks.find((task) => task.id === selection.taskId)
    ?? filteredTasks[0]
    ?? data.tasks[0];
  if (!selectedTask) return null;
  const taskCheckpoints = selectedTask.checkpointIds
    .map((id) => data.checkpoints.find((checkpoint) => checkpoint.id === id))
    .filter((checkpoint): checkpoint is Checkpoint => Boolean(checkpoint));
  const selectedCheckpoint = taskCheckpoints.find((checkpoint) => checkpoint.id === selection.checkpointId)
    ?? taskCheckpoints[0];
  const explicitlySelectedEvidence = data.evidence.find((evidence) => evidence.id === selection.evidenceId);
  const taskEvidence = data.evidence.filter((evidence) => selectedTask.checkpointIds.includes(evidence.checkpointId));
  const selectedEvidence = selectedCheckpoint
    ? (explicitlySelectedEvidence?.checkpointId === selectedCheckpoint.id
        && selectedTask.checkpointIds.includes(explicitlySelectedEvidence.checkpointId)
        ? explicitlySelectedEvidence
        : data.evidence.find((evidence) => evidence.checkpointId === selectedCheckpoint.id))
    : undefined;
  const selectedFact = data.facts.find((fact) => fact.id === selectedEvidence?.factId && fact.taskId === selectedTask.id)
    ?? data.facts.find((fact) => fact.taskId === selectedTask.id);
  const resumeCandidate = data.resumeCandidate?.taskId === selectedTask.id ? data.resumeCandidate : undefined;
  const selectedContractEvaluation = data.contractEvaluations.find(
    (evaluation) => evaluation.taskId === selectedTask.id,
  ) ?? (data.contractEvaluation?.taskId === selectedTask.id ? data.contractEvaluation : null);

  return {
    selectedTask,
    taskCheckpoints,
    selectedCheckpoint,
    selectedEvidence,
    taskEvidence,
    selectedFact,
    evidenceAvailable: Boolean(selectedEvidence && selectedFact),
    resumeCandidate,
    selectedContractEvaluation,
  };
}

function inferRepositoryState(bootstrap: BootstrapData): BootstrapData['repository']['state'] {
  if (!bootstrap.repository.connected) return 'unregistered';
  if (bootstrap.repository.captureHealth === 'degraded' || bootstrap.repository.captureHealth === 'offline') return 'degraded';
  return bootstrap.checkpoints?.length > 0 ? 'active' : 'registered-empty';
}
