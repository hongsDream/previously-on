import { useDeferredValue, useMemo, useState } from 'react';
import { AppHeader } from './components/AppHeader';
import { BottomNavigation } from './components/BottomNavigation';
import { EvidenceInspector } from './components/EvidenceInspector';
import { FirstRunSetup, RegisteredEmptyActions } from './components/FirstRunSetup';
import { ProjectOverview } from './components/ProjectOverview';
import { Sidebar } from './components/Sidebar';
import { SettingsPanel } from './components/SettingsPanel';
import { TaskWorkspace } from './components/TaskWorkspace';
import { RegressionContractsPanel } from './components/RegressionContractsPanel';
import { useBootstrap } from './hooks/useBootstrap';
import { useWorkspaceNavigation } from './hooks/useWorkspaceNavigation';
import {
  exportRepository,
  fetchFactRefresh,
  fetchBootstrap,
  approveContractCandidate,
  applyTaskGrouping,
  createContractCandidate,
  previewTaskGrouping,
  purgeRepository,
  revalidateFact,
  reviewFactRefreshCandidate,
  startFactRefresh,
  supersedeRegressionContract,
  updateContractCandidate,
  updateFactStatus,
  updateFact,
  updateSession,
  undoTaskGrouping,
  updateTask,
} from './lib/api';
import type { ContractMutationResponse, FactCandidateReviewResponse } from './lib/api';
import {
  emptyWorkspaceSelection,
  resolveTaskSelection,
  selectWorkspace,
} from './lib/workspace';
import type {
  AiFactRefreshOperationV1,
  BootstrapData,
  Checkpoint,
  FactStatus,
  Fact,
  FactKind,
  RegressionCandidateDraftV1,
  TaskGroupingPreviewV1,
  TaskGroupingRequestV1,
  TaskStatus,
  TaskUpdateV1,
} from './types';

export function App() {
  const {
    data,
    setData,
    offlineFallback,
    fatalError,
    selection,
    setSelection,
    installBootstrap,
  } = useBootstrap();
  const {
    workspaceView,
    overviewFocus,
    contextPackExpanded,
    mobileInspectorOpen,
    openTask,
    openOverview,
    openSettings,
    showContextPack,
    toggleContextPack,
    showEvidence,
    openInspector,
    closeInspector,
  } = useWorkspaceNavigation();
  const [query, setQuery] = useState('');
  const [status, setStatus] = useState<TaskStatus | 'all'>('all');
  const [mutationPending, setMutationPending] = useState(false);
  const [actionError, setActionError] = useState('');
  const [graphRefreshVersion, setGraphRefreshVersion] = useState(0);
  const deferredQuery = useDeferredValue(query);

  const filteredTasks = useMemo(() => {
    if (!data) return [];
    const normalized = deferredQuery.trim().toLowerCase();
    return data.tasks.filter((task) => {
      const matchesStatus = status === 'all' || task.status === status;
      const matchesQuery = normalized.length === 0 || task.title.toLowerCase().includes(normalized) || task.goal.toLowerCase().includes(normalized);
      return matchesStatus && matchesQuery;
    });
  }, [data, deferredQuery, status]);

  if (fatalError) return <ErrorScreen message={fatalError} />;
  if (!data) return <LoadingScreen />;
  const currentData = data;
  const isUnregistered = currentData.repository.state === 'unregistered';

  const performMutation = async <T,>(mutation: () => Promise<T>): Promise<T | null> => {
    if (offlineFallback || isUnregistered || mutationPending) return null;
    setMutationPending(true);
    setActionError('');
    try {
      return await mutation();
    } catch (error) {
      console.error('PreviouslyOn mutation failed', error);
      setActionError(error instanceof Error ? error.message : 'The local change could not be saved.');
      return null;
    } finally {
      setMutationPending(false);
    }
  };

  async function handleBootstrapRefresh() {
    if (offlineFallback || mutationPending) return;
    setMutationPending(true);
    setActionError('');
    try {
      installBootstrap(await fetchBootstrap(), selection);
    } catch (error) {
      setActionError(error instanceof Error ? error.message : 'The local status could not be refreshed.');
    } finally {
      setMutationPending(false);
    }
  }

  async function handleExport() {
    if (offlineFallback || isUnregistered || mutationPending) return;
    setActionError('');
    try {
      const exported = await exportRepository();
      const blob = new Blob([`${JSON.stringify(exported, null, 2)}\n`], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const link = document.createElement('a');
      const safeName = currentData.repository.name.replace(/[^a-zA-Z0-9._-]+/g, '-');
      link.href = url;
      link.download = `${safeName || 'previously-on'}-export.json`;
      link.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      setActionError(error instanceof Error ? error.message : 'The export could not be created.');
    }
  }

  async function handlePurge() {
    if (offlineFallback || isUnregistered || mutationPending) return;
    const confirmed = window.confirm(`Permanently delete all PreviouslyOn data for ${currentData.repository.path}? This cannot be undone.`);
    if (!confirmed) return;
    const purged = await performMutation(purgeRepository);
    if (purged !== null) {
      setData((current) => current ? {
        ...current,
        repository: {
          ...current.repository,
          state: 'registered-empty',
          captureHealth: 'good',
        },
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
  }

  async function handleCreateContractCandidate(draft: RegressionCandidateDraftV1) {
    const result = await performMutation(() => createContractCandidate(draft));
    if (!result) return false;
    setData((current) => current ? mergeContractMutation(current, result) : current);
    return true;
  }

  async function handleUpdateContractCandidate(id: string, draft: RegressionCandidateDraftV1) {
    const previous = currentData.contractCandidates.find((candidate) => candidate.id === id);
    setData((current) => current ? {
      ...current,
      contractCandidates: current.contractCandidates.map((candidate) => candidate.id === id ? { ...candidate, ...draft } : candidate),
    } : current);
    const result = await performMutation(() => updateContractCandidate(id, draft));
    if (!result) {
      if (previous) {
        setData((current) => current ? {
          ...current,
          contractCandidates: current.contractCandidates.map((candidate) => candidate.id === id ? previous : candidate),
        } : current);
      }
      return false;
    }
    setData((current) => current ? mergeContractMutation(current, result) : current);
    return true;
  }

  async function handleApproveContractCandidate(id: string) {
    const result = await performMutation(() => approveContractCandidate(id));
    if (!result) return false;
    setData((current) => current ? mergeContractMutation({
      ...current,
      contractCandidates: current.contractCandidates.filter((candidate) => candidate.id !== id),
    }, result) : current);
    return true;
  }

  async function handleSupersedeContract(id: string, supersededBy: string) {
    const previous = currentData.contracts.find((contract) => contract.id === id);
    setData((current) => current ? {
      ...current,
      contracts: current.contracts.map((contract) => contract.id === id ? { ...contract, status: 'superseded', supersededBy } : contract),
    } : current);
    const result = await performMutation(() => supersedeRegressionContract(id, supersededBy));
    if (!result) {
      if (previous) {
        setData((current) => current ? {
          ...current,
          contracts: current.contracts.map((contract) => contract.id === id ? previous : contract),
        } : current);
      }
      return false;
    }
    setData((current) => current ? mergeContractMutation(current, result) : current);
    return true;
  }

  if (data.tasks.length === 0) {
    return (
      <div className="app-shell">
        <AppHeader
          repository={data.repository}
          onPreview={() => undefined}
          onExport={() => void handleExport()}
          onPurge={() => void handlePurge()}
          actionsDisabled={offlineFallback || isUnregistered || mutationPending}
          previewDisabled
        />
        {actionError ? <div className="action-error" role="alert">{actionError}</div> : null}
        <div className="app-body empty-app-body">
          <Sidebar
            query={query}
            status={status}
            tasks={[]}
            selectedTaskId=""
            onQueryChange={setQuery}
            onStatusChange={setStatus}
            onTaskSelect={() => undefined}
            activeNavigation={workspaceView === 'settings' ? 'settings' : 'tasks'}
            onOverviewOpen={openTask}
            onEvidenceOpen={openInspector}
            evidenceEnabled={false}
            onSettingsOpen={openSettings}
          />
          {workspaceView === 'settings' ? <SettingsPanel capability={data.aiRefreshCapability} /> : <main className="repository-empty-workspace">
            {isUnregistered ? <FirstRunSetup /> : null}
            <RegressionContractsPanel
              contracts={data.contracts}
              candidates={data.contractCandidates}
              evaluation={data.contractEvaluation}
              disabled={offlineFallback || isUnregistered}
              mutationPending={mutationPending}
              onCreateCandidate={handleCreateContractCandidate}
              onUpdateCandidate={handleUpdateContractCandidate}
              onApproveCandidate={handleApproveContractCandidate}
              onSupersedeContract={handleSupersedeContract}
            />
            {isUnregistered ? null : data.repository.state === 'degraded'
              ? <DegradedWorkspace repositoryName={data.repository.name} />
              : <EmptyWorkspace
                  repositoryName={data.repository.name}
                  repositoryPath={data.repository.path}
                  refreshPending={mutationPending}
                  onRefresh={() => void handleBootstrapRefresh()}
                />}
          </main>}
        </div>
        <BottomNavigation
          activeNavigation={workspaceView === 'settings' ? 'settings' : 'tasks'}
          sessionsEnabled={false}
          evidenceEnabled={false}
          onTasksOpen={openTask}
          onSessionsOpen={() => undefined}
          onEvidenceOpen={openInspector}
          onSettingsOpen={openSettings}
        />
      </div>
    );
  }

  const workspace = selectWorkspace(data, selection, filteredTasks)!;
  const {
    selectedTask,
    taskCheckpoints,
    selectedCheckpoint,
    selectedEvidence,
    taskEvidence,
    selectedFact,
    evidenceAvailable,
    resumeCandidate,
    selectedContractEvaluation,
  } = workspace;

  const selectTask = (taskId: string) => {
    const selection = resolveTaskSelection(data, taskId, data.tasks.find((item) => item.id === taskId)?.checkpointIds[0]);
    if (!selection.taskId) return;
    setSelection(selection);
    openTask();
  };

  const openContextPack = () => {
    if (!selectedCheckpoint || !data.contextPacks[selectedTask.id]) return;
    showContextPack();
  };

  const openEvidence = () => {
    if (!selectedEvidence) return;
    showEvidence();
  };

  const selectCheckpoint = (checkpoint: Checkpoint) => {
    const matchingEvidence = data.evidence.find((evidence) => evidence.checkpointId === checkpoint.id);
    setSelection((current) => ({
      ...current,
      checkpointId: checkpoint.id,
      evidenceId: matchingEvidence?.id ?? current.evidenceId,
    }));
    openInspector();
  };

  const selectEvidence = (evidenceId: string) => {
    const evidence = data.evidence.find((item) => item.id === evidenceId);
    if (!evidence || !selectedTask.checkpointIds.includes(evidence.checkpointId)) return;
    setSelection((current) => ({
      ...current,
      evidenceId: evidence.id,
      checkpointId: evidence.checkpointId || current.checkpointId,
    }));
    openInspector();
  };

  const handleFactStatus = async (nextStatus: FactStatus, supersedesFactId?: string) => {
    if (offlineFallback || mutationPending || !selectedFact) return;
    const previousStatus = selectedFact.status;
    setData((current) => current ? {
      ...current,
      facts: current.facts.map((fact) => fact.id === selectedFact.id ? { ...fact, status: nextStatus } : fact),
    } : current);
    const saved = await performMutation(() => updateFactStatus(selectedFact.id, nextStatus, supersedesFactId));
    if (saved === null) {
      setData((current) => current ? {
        ...current,
        facts: current.facts.map((fact) => fact.id === selectedFact.id ? { ...fact, status: previousStatus } : fact),
      } : current);
    }
  };

  const handleFactUpdate = async (content: string, deprecatedAfterCommit: string) => {
    if (offlineFallback || mutationPending || !selectedFact) return false;
    const previous = selectedFact;
    setData((current) => current ? {
      ...current,
      facts: current.facts.map((fact) => fact.id === selectedFact.id ? { ...fact, text: content, deprecatedAfterCommit: deprecatedAfterCommit || undefined } : fact),
    } : current);
    const saved = await performMutation(() => updateFact(selectedFact.id, selectedFact.status, content, deprecatedAfterCommit));
    if (!saved) {
      setData((current) => current ? {
        ...current,
        facts: current.facts.map((fact) => fact.id === selectedFact.id ? previous : fact),
      } : current);
      return false;
    }
    setData((current) => current ? {
      ...current,
      facts: current.facts.map((fact) => fact.id === selectedFact.id ? {
        ...fact,
        text: saved.text,
        updatedAt: saved.updatedAt,
        deprecatedAfterCommit: saved.deprecatedAfterCommit || undefined,
      } : fact),
    } : current);
    return true;
  };

  const handleSessionExcluded = async (excluded: boolean) => {
    if (offlineFallback || mutationPending || !selectedEvidence?.sessionId) return;
    const sessionId = selectedEvidence.sessionId;
    const saved = await performMutation(() => updateSession(sessionId, excluded));
    if (!saved) return;
    setData((current) => current ? {
      ...current,
      sessions: current.sessions.map((session) => session.id === sessionId ? { ...session, excluded: saved.excluded } : session),
      evidence: current.evidence.map((evidence) => evidence.sessionId === sessionId ? { ...evidence, excludedSession: saved.excluded } : evidence),
    } : current);
  };

  const dismissCandidate = () => {
    if (!resumeCandidate || offlineFallback) return;
    setData((current) => current ? { ...current, resumeCandidate: undefined } : current);
  };

  const reviewCandidate = () => {
    if (!resumeCandidate) return;
    const recommended = taskCheckpoints.find((checkpoint) => checkpoint.sequence === 2) ?? taskCheckpoints[0];
    if (recommended) selectCheckpoint(recommended);
    showContextPack();
  };

  const handleRevalidate = async () => {
    if (offlineFallback || mutationPending || !selectedFact) return;
    const result = await performMutation(() => revalidateFact(selectedFact.id));
    if (!result) return;
    setData((current) => current ? {
      ...current,
      evidence: current.evidence.map((evidence) => selectedFact.evidenceIds.includes(evidence.id)
        ? { ...evidence, freshness: result.freshness }
        : evidence),
      facts: current.facts.map((fact) => fact.id === selectedFact.id
        ? { ...fact, updatedAt: result.validatedAt }
        : fact),
    } : current);
  };

  const installRefreshedBootstrap = (next: BootstrapData, preferredTaskId = selectedTask.id) => {
    installBootstrap(next, {
      taskId: preferredTaskId,
      checkpointId: selection.checkpointId,
      evidenceId: selection.evidenceId,
    });
    setGraphRefreshVersion((version) => version + 1);
  };

  const mutateAndRefresh = async (mutation: () => Promise<unknown>, preferredTaskId = selectedTask.id) => {
    if (offlineFallback || mutationPending) return false;
    const refreshed = await performMutation(async () => {
      await mutation();
      return fetchBootstrap();
    });
    if (!refreshed) return false;
    installRefreshedBootstrap(refreshed, preferredTaskId);
    return true;
  };

  const handleTaskUpdate = (update: TaskUpdateV1) => mutateAndRefresh(
    () => updateTask(selectedTask.id, update),
    selectedTask.id,
  );

  const handleGroupingPreview = async (request: TaskGroupingRequestV1): Promise<TaskGroupingPreviewV1 | null> => {
    if (offlineFallback || mutationPending) return null;
    return performMutation(() => previewTaskGrouping(request));
  };

  const handleGroupingApply = (request: TaskGroupingRequestV1) => mutateAndRefresh(
    () => applyTaskGrouping(request),
    request.fromTaskId,
  );

  const handleGroupingUndo = (operationId: string) => mutateAndRefresh(
    () => undoTaskGrouping(operationId),
    selectedTask.id,
  );

  const installFactRefreshOperation = (operation: AiFactRefreshOperationV1) => {
    setData((current) => current ? {
      ...current,
      factRefreshOperations: current.factRefreshOperations.some((item) => item.operationId === operation.operationId)
        ? current.factRefreshOperations.map((item) => item.operationId === operation.operationId ? operation : item)
        : [...current.factRefreshOperations, operation],
    } : current);
  };

  const handleFactRefreshStart = async (requestId: string): Promise<AiFactRefreshOperationV1 | null> => {
    if (offlineFallback || mutationPending || currentData.aiRefreshCapability.status !== 'ready') return null;
    const operation = await performMutation(() => startFactRefresh(selectedTask.id, requestId));
    if (operation) installFactRefreshOperation(operation);
    return operation;
  };

  const handleFactRefreshPoll = async (operationId: string, signal: AbortSignal): Promise<AiFactRefreshOperationV1 | null> => {
    if (offlineFallback) return null;
    try {
      const operation = await fetchFactRefresh(operationId, signal);
      installFactRefreshOperation(operation);
      return operation;
    } catch (error) {
      if (error instanceof DOMException && error.name === 'AbortError') return null;
      console.error('PreviouslyOn fact refresh polling failed', error);
      setActionError(error instanceof Error ? error.message : 'The local refresh status could not be checked.');
      return null;
    }
  };

  const handleFactRefreshReview = async (
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
  };

  return (
    <div className="app-shell">
      <AppHeader
        repository={data.repository}
        onPreview={openContextPack}
        onExport={() => void handleExport()}
        onPurge={() => void handlePurge()}
        actionsDisabled={offlineFallback || mutationPending}
        previewDisabled={!selectedCheckpoint || !data.contextPacks[selectedTask.id]}
      />
      {offlineFallback ? <div className="sample-banner" role="status">Local API unavailable · read-only sample workspace · changes are disabled</div> : null}
      {!offlineFallback && data.repository.state === 'degraded' ? <div className="degraded-banner" role="status">Capture degraded · review missing evidence before trusting this workspace</div> : null}
      {actionError ? <div className="action-error" role="alert">{actionError}</div> : null}
      <div className={`app-body ${workspaceView === 'overview' || workspaceView === 'settings' ? 'overview-app-body' : ''}`}>
        <Sidebar
          query={query}
          status={status}
          tasks={filteredTasks}
          selectedTaskId={selectedTask.id}
          onQueryChange={setQuery}
          onStatusChange={setStatus}
          onTaskSelect={selectTask}
          activeNavigation={workspaceView === 'overview' ? overviewFocus : workspaceView}
          onOverviewOpen={openOverview}
          onEvidenceOpen={openEvidence}
          evidenceEnabled={evidenceAvailable}
          onSettingsOpen={openSettings}
        />
        {workspaceView === 'settings' ? <SettingsPanel capability={data.aiRefreshCapability} /> : workspaceView === 'overview' ? (
          <ProjectOverview
            tasks={filteredTasks}
            graphTasks={data.tasks}
            sessions={data.sessions}
            facts={data.facts}
            repositoryId={data.tasks[0]?.repositoryId ?? ''}
            graphSummary={data.graphSummary}
            graphRefreshVersion={graphRefreshVersion}
            graphDisabled={offlineFallback}
            focus={overviewFocus}
            onTaskSelect={selectTask}
          />
        ) : <TaskWorkspace
          task={selectedTask}
          tasks={data.tasks}
          sessions={data.sessions}
          facts={data.facts}
          groupingOperations={data.taskGroupingOperations}
          aiRefreshCapability={data.aiRefreshCapability}
          factRefreshOperation={latestFactRefreshOperation(data.factRefreshOperations, selectedTask.id)}
          agents={data.agents}
          checkpoints={taskCheckpoints}
          selectedCheckpoint={selectedCheckpoint}
          resumeCandidate={resumeCandidate}
          contextPack={data.contextPacks[selectedTask.id]}
          contracts={data.contracts}
          contractCandidates={data.contractCandidates}
          contractEvaluation={selectedContractEvaluation}
          contextPackExpanded={contextPackExpanded}
          onCheckpointSelect={selectCheckpoint}
          onReviewResume={reviewCandidate}
          onDismissResume={dismissCandidate}
          onToggleContextPack={toggleContextPack}
          onTaskUpdate={handleTaskUpdate}
          onGroupingPreview={handleGroupingPreview}
          onGroupingApply={handleGroupingApply}
          onGroupingUndo={handleGroupingUndo}
          onFactRefreshStart={handleFactRefreshStart}
          onFactRefreshPoll={handleFactRefreshPoll}
          onFactRefreshReview={handleFactRefreshReview}
          onCreateContractCandidate={handleCreateContractCandidate}
          onUpdateContractCandidate={handleUpdateContractCandidate}
          onApproveContractCandidate={handleApproveContractCandidate}
          onSupersedeContract={handleSupersedeContract}
          contractMutationsDisabled={offlineFallback}
          mutationPending={mutationPending}
          onBack={() => openOverview('tasks')}
        />}
        {workspaceView === 'task' && selectedEvidence && selectedFact ? (
          <EvidenceInspector
            evidence={selectedEvidence}
            availableEvidence={taskEvidence}
            fact={selectedFact}
            replacementFacts={data.facts.filter((fact) => fact.taskId === selectedTask.id && fact.id !== selectedFact.id && !['invalid', 'superseded'].includes(fact.status))}
            mutationPending={mutationPending}
            mobileOpen={mobileInspectorOpen}
            onClose={closeInspector}
            onEvidenceSelect={selectEvidence}
            onStatusChange={(nextStatus, supersedesFactId) => void handleFactStatus(nextStatus, supersedesFactId)}
            onFactUpdate={handleFactUpdate}
            onSessionExcludedChange={(excluded) => void handleSessionExcluded(excluded)}
            onRevalidate={() => void handleRevalidate()}
          />
        ) : null}
      </div>
      <BottomNavigation
        activeNavigation={workspaceView === 'settings' ? 'settings' : workspaceView === 'overview' ? overviewFocus : 'tasks'}
        sessionsEnabled={data.sessions.length > 0}
        evidenceEnabled={evidenceAvailable}
        onTasksOpen={() => openOverview('tasks')}
        onSessionsOpen={() => openOverview('sessions')}
        onEvidenceOpen={openEvidence}
        onSettingsOpen={openSettings}
      />
    </div>
  );
}

function ErrorScreen({ message }: { message: string }) {
  return (
    <main className="loading-screen" role="alert">
      <span className="loading-mark error-mark" />
      <h1>PreviouslyOn could not load</h1>
      <p>{message}</p>
    </main>
  );
}

function LoadingScreen() {
  return (
    <main className="loading-screen" aria-busy="true">
      <span className="loading-mark" />
      <p>Loading PreviouslyOn…</p>
    </main>
  );
}

function EmptyWorkspace({ repositoryName, repositoryPath, refreshPending, onRefresh }: {
  repositoryName: string;
  repositoryPath: string;
  refreshPending: boolean;
  onRefresh: () => void;
}) {
  return (
    <section className="repository-empty-state">
      <span className="empty-lineage-mark" aria-hidden="true" />
      <h1>{repositoryName} is connected</h1>
      <p>Start one captured Codex session, finish it normally, then refresh this screen to review the first checkpoint and its evidence.</p>
      <RegisteredEmptyActions repositoryPath={repositoryPath} refreshPending={refreshPending} onRefresh={onRefresh} />
    </section>
  );
}

function DegradedWorkspace({ repositoryName }: { repositoryName: string }) {
  return (
    <section className="repository-empty-state degraded-empty-state">
      <span className="empty-lineage-mark" aria-hidden="true" />
      <h1>{repositoryName} capture is degraded</h1>
      <p>PreviouslyOn found the registered repository, but it cannot confirm a complete first checkpoint. Run <code>previously doctor</code>, then start a new captured session after resolving the reported issue.</p>
    </section>
  );
}

function latestFactRefreshOperation(operations: AiFactRefreshOperationV1[], taskId: string) {
  return operations
    .filter((operation) => operation.taskId === taskId)
    .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt))[0];
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

function mergeContractMutation(current: BootstrapData, response: ContractMutationResponse): BootstrapData {
  let contracts = response.contracts ?? current.contracts;
  let contractCandidates = response.contractCandidates ?? current.contractCandidates;
  if (!response.contracts && response.contract) {
    const exists = contracts.some((contract) => contract.id === response.contract?.id);
    contracts = exists
      ? contracts.map((contract) => contract.id === response.contract?.id ? response.contract! : contract)
      : [...contracts, response.contract];
  }
  if (!response.contractCandidates && response.candidate) {
    const exists = contractCandidates.some((candidate) => candidate.id === response.candidate?.id);
    contractCandidates = exists
      ? contractCandidates.map((candidate) => candidate.id === response.candidate?.id ? response.candidate! : candidate)
      : [...contractCandidates, response.candidate];
  }
  return {
    ...current,
    contracts,
    contractCandidates,
    contractEvaluation: response.contractEvaluation === undefined ? current.contractEvaluation : response.contractEvaluation,
    contractEvaluations: current.contractEvaluations,
  };
}
