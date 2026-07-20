import { useDeferredValue, useEffect, useMemo, useState } from 'react';
import { AppHeader } from './components/AppHeader';
import { BottomNavigation } from './components/BottomNavigation';
import { EvidenceInspector } from './components/EvidenceInspector';
import { FirstRunSetup, RegisteredEmptyActions } from './components/FirstRunSetup';
import { ProjectOverview } from './components/ProjectOverview';
import { Sidebar } from './components/Sidebar';
import { SettingsPanel } from './components/SettingsPanel';
import { TaskWorkspace } from './components/TaskWorkspace';
import { RegressionContractsPanel } from './components/RegressionContractsPanel';
import { fallbackData } from './data/fallback';
import {
  exportRepository,
  fetchFactRefresh,
  fetchBootstrap,
  ApiUnavailableError,
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
  const [data, setData] = useState<BootstrapData | null>(null);
  const [offlineFallback, setOfflineFallback] = useState(false);
  const [fatalError, setFatalError] = useState('');
  const [selectedTaskId, setSelectedTaskId] = useState('');
  const [selectedCheckpointId, setSelectedCheckpointId] = useState('');
  const [selectedEvidenceId, setSelectedEvidenceId] = useState('');
  const [workspaceView, setWorkspaceView] = useState<'overview' | 'task' | 'settings'>('task');
  const [overviewFocus, setOverviewFocus] = useState<'tasks' | 'sessions'>('tasks');
  const [query, setQuery] = useState('');
  const [status, setStatus] = useState<TaskStatus | 'all'>('all');
  const [contextPackExpanded, setContextPackExpanded] = useState(() => (
    typeof window.matchMedia !== 'function' || !window.matchMedia('(max-width: 900px)').matches
  ));
  const [mobileInspectorOpen, setMobileInspectorOpen] = useState(true);
  const [mutationPending, setMutationPending] = useState(false);
  const [actionError, setActionError] = useState('');
  const [graphRefreshVersion, setGraphRefreshVersion] = useState(0);
  const deferredQuery = useDeferredValue(query);

  useEffect(() => {
    const controller = new AbortController();
    fetchBootstrap(controller.signal)
      .then((bootstrap) => {
        const normalized = normalizeBootstrap(bootstrap);
        const selection = resolveTaskSelection(normalized);
        setData(normalized);
        setSelectedTaskId(selection.taskId);
        setSelectedCheckpointId(selection.checkpointId);
        setSelectedEvidenceId(selection.evidenceId);
      })
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === 'AbortError') return;
        if (!(error instanceof ApiUnavailableError)) {
          setFatalError(error instanceof Error ? error.message : 'The local API returned an invalid response.');
          return;
        }
        const selection = resolveTaskSelection(fallbackData);
        setOfflineFallback(true);
        setData(fallbackData);
        setSelectedTaskId(selection.taskId);
        setSelectedCheckpointId(selection.checkpointId);
        setSelectedEvidenceId(selection.evidenceId);
      });
    return () => controller.abort();
  }, []);

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
      const normalized = normalizeBootstrap(await fetchBootstrap());
      const selection = resolveTaskSelection(normalized, selectedTaskId, selectedCheckpointId, selectedEvidenceId);
      setData(normalized);
      setSelectedTaskId(selection.taskId);
      setSelectedCheckpointId(selection.checkpointId);
      setSelectedEvidenceId(selection.evidenceId);
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
      setSelectedTaskId('');
      setSelectedCheckpointId('');
      setSelectedEvidenceId('');
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
            onOverviewOpen={() => setWorkspaceView('task')}
            onEvidenceOpen={() => setMobileInspectorOpen(true)}
            evidenceEnabled={false}
            onSettingsOpen={() => setWorkspaceView('settings')}
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
          onTasksOpen={() => setWorkspaceView('task')}
          onSessionsOpen={() => undefined}
          onEvidenceOpen={() => setMobileInspectorOpen(true)}
          onSettingsOpen={() => setWorkspaceView('settings')}
        />
      </div>
    );
  }

  const selectedTask = data.tasks.find((task) => task.id === selectedTaskId) ?? filteredTasks[0] ?? data.tasks[0];
  const taskCheckpoints = selectedTask.checkpointIds
    .map((id) => data.checkpoints.find((checkpoint) => checkpoint.id === id))
    .filter((checkpoint): checkpoint is Checkpoint => Boolean(checkpoint));
  const selectedCheckpoint = taskCheckpoints.find((checkpoint) => checkpoint.id === selectedCheckpointId) ?? taskCheckpoints[0];
  const explicitlySelectedEvidence = data.evidence.find((evidence) => evidence.id === selectedEvidenceId);
  const taskEvidence = data.evidence.filter((evidence) => selectedTask.checkpointIds.includes(evidence.checkpointId));
  const selectedEvidence = selectedCheckpoint
    ? (explicitlySelectedEvidence?.checkpointId === selectedCheckpoint.id && selectedTask.checkpointIds.includes(explicitlySelectedEvidence.checkpointId)
        ? explicitlySelectedEvidence
        : data.evidence.find((evidence) => evidence.checkpointId === selectedCheckpoint.id))
    : undefined;
  const selectedFact = data.facts.find((fact) => fact.id === selectedEvidence?.factId && fact.taskId === selectedTask.id)
    ?? data.facts.find((fact) => fact.taskId === selectedTask.id);
  const evidenceAvailable = Boolean(selectedEvidence && selectedFact);
  const resumeCandidate = data.resumeCandidate?.taskId === selectedTask.id ? data.resumeCandidate : undefined;
  const selectedContractEvaluation = data.contractEvaluations.find(
    (evaluation) => evaluation.taskId === selectedTask.id,
  ) ?? (data.contractEvaluation?.taskId === selectedTask.id ? data.contractEvaluation : null);

  const selectTask = (taskId: string) => {
    const selection = resolveTaskSelection(data, taskId, data.tasks.find((item) => item.id === taskId)?.checkpointIds[0]);
    if (!selection.taskId) return;
    setSelectedTaskId(selection.taskId);
    setSelectedCheckpointId(selection.checkpointId);
    setSelectedEvidenceId(selection.evidenceId);
    setWorkspaceView('task');
  };

  const openOverview = (focus: 'tasks' | 'sessions') => {
    setOverviewFocus(focus);
    setWorkspaceView('overview');
    setMobileInspectorOpen(false);
  };

  const openSettings = () => {
    setWorkspaceView('settings');
    setMobileInspectorOpen(false);
  };

  const openContextPack = () => {
    if (!selectedCheckpoint || !data.contextPacks[selectedTask.id]) return;
    setWorkspaceView('task');
    setContextPackExpanded(true);
  };

  const openEvidence = () => {
    if (!selectedEvidence) return;
    setWorkspaceView('task');
    setMobileInspectorOpen(true);
  };

  const selectCheckpoint = (checkpoint: Checkpoint) => {
    setSelectedCheckpointId(checkpoint.id);
    const matchingEvidence = data.evidence.find((evidence) => evidence.checkpointId === checkpoint.id);
    if (matchingEvidence) setSelectedEvidenceId(matchingEvidence.id);
    setMobileInspectorOpen(true);
  };

  const selectEvidence = (evidenceId: string) => {
    const evidence = data.evidence.find((item) => item.id === evidenceId);
    if (!evidence || !selectedTask.checkpointIds.includes(evidence.checkpointId)) return;
    setSelectedEvidenceId(evidence.id);
    if (evidence.checkpointId) setSelectedCheckpointId(evidence.checkpointId);
    setMobileInspectorOpen(true);
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
    setContextPackExpanded(true);
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
    const normalized = normalizeBootstrap(next);
    const selection = resolveTaskSelection(normalized, preferredTaskId, selectedCheckpointId, selectedEvidenceId);
    setData(normalized);
    setSelectedTaskId(selection.taskId);
    setSelectedCheckpointId(selection.checkpointId);
    setSelectedEvidenceId(selection.evidenceId);
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
          onToggleContextPack={() => setContextPackExpanded((expanded) => !expanded)}
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
            onClose={() => setMobileInspectorOpen(false)}
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

function normalizeBootstrap(bootstrap: BootstrapData): BootstrapData {
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
      reason: 'The local API did not provide a verified AI refresh capability.',
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

function inferRepositoryState(bootstrap: BootstrapData): BootstrapData['repository']['state'] {
  if (!bootstrap.repository.connected) return 'unregistered';
  if (bootstrap.repository.captureHealth === 'degraded' || bootstrap.repository.captureHealth === 'offline') return 'degraded';
  return bootstrap.checkpoints?.length > 0 ? 'active' : 'registered-empty';
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

function resolveTaskSelection(
  data: BootstrapData,
  preferredTaskId?: string,
  preferredCheckpointId?: string,
  preferredEvidenceId?: string,
) {
  const task = data.tasks.find((candidate) => candidate.id === preferredTaskId) ?? data.tasks[0];
  if (!task) return { taskId: '', checkpointId: '', evidenceId: '' };
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
