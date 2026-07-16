import { useDeferredValue, useEffect, useMemo, useState } from 'react';
import { AppHeader } from './components/AppHeader';
import { BottomNavigation } from './components/BottomNavigation';
import { EvidenceInspector } from './components/EvidenceInspector';
import { ProjectOverview } from './components/ProjectOverview';
import { Sidebar } from './components/Sidebar';
import { TaskWorkspace } from './components/TaskWorkspace';
import { RegressionContractsPanel } from './components/RegressionContractsPanel';
import { fallbackData } from './data/fallback';
import {
  exportRepository,
  fetchBootstrap,
  ApiUnavailableError,
  approveContractCandidate,
  createContractCandidate,
  purgeRepository,
  revalidateFact,
  supersedeRegressionContract,
  updateContractCandidate,
  updateFactStatus,
  updateFact,
  updateSession,
  updateTaskStatus,
} from './lib/api';
import type { ContractMutationResponse } from './lib/api';
import type { BootstrapData, Checkpoint, FactStatus, RegressionCandidateDraftV1, TaskStatus } from './types';

export function App() {
  const [data, setData] = useState<BootstrapData | null>(null);
  const [offlineFallback, setOfflineFallback] = useState(false);
  const [fatalError, setFatalError] = useState('');
  const [selectedTaskId, setSelectedTaskId] = useState('');
  const [selectedCheckpointId, setSelectedCheckpointId] = useState('');
  const [selectedEvidenceId, setSelectedEvidenceId] = useState('');
  const [workspaceView, setWorkspaceView] = useState<'overview' | 'task'>('task');
  const [overviewFocus, setOverviewFocus] = useState<'tasks' | 'sessions'>('tasks');
  const [query, setQuery] = useState('');
  const [status, setStatus] = useState<TaskStatus | 'all'>('all');
  const [contextPackExpanded, setContextPackExpanded] = useState(() => (
    typeof window.matchMedia !== 'function' || !window.matchMedia('(max-width: 900px)').matches
  ));
  const [mobileInspectorOpen, setMobileInspectorOpen] = useState(true);
  const [mutationPending, setMutationPending] = useState(false);
  const [actionError, setActionError] = useState('');
  const deferredQuery = useDeferredValue(query);

  useEffect(() => {
    const controller = new AbortController();
    fetchBootstrap(controller.signal)
      .then((bootstrap) => {
        const normalized = normalizeBootstrap(bootstrap);
        setData(normalized);
        const task = normalized.tasks[0];
        const checkpointId = task?.checkpointIds[1] ?? task?.checkpointIds[0] ?? '';
        setSelectedTaskId(task?.id ?? '');
        setSelectedCheckpointId(checkpointId);
        setSelectedEvidenceId(normalized.evidence.find((item) => item.checkpointId === checkpointId)?.id ?? normalized.evidence[0]?.id ?? '');
      })
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === 'AbortError') return;
        if (!(error instanceof ApiUnavailableError)) {
          setFatalError(error instanceof Error ? error.message : 'The local API returned an invalid response.');
          return;
        }
        const task = fallbackData.tasks[0];
        const checkpointId = task.checkpointIds[1];
        setOfflineFallback(true);
        setData(fallbackData);
        setSelectedTaskId(task.id);
        setSelectedCheckpointId(checkpointId);
        setSelectedEvidenceId(fallbackData.evidence.find((item) => item.checkpointId === checkpointId)?.id ?? fallbackData.evidence[0].id);
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

  const performMutation = async <T,>(mutation: () => Promise<T>): Promise<T | null> => {
    if (offlineFallback || mutationPending) return null;
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

  async function handleExport() {
    if (offlineFallback || mutationPending) return;
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
    if (offlineFallback || mutationPending) return;
    const confirmed = window.confirm(`Permanently delete all PreviouslyOn data for ${currentData.repository.path}? This cannot be undone.`);
    if (!confirmed) return;
    const purged = await performMutation(purgeRepository);
    if (purged !== null) {
      setData((current) => current ? {
        ...current,
        tasks: [],
        checkpoints: [],
        facts: [],
        evidence: [],
        sessions: [],
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
          actionsDisabled={offlineFallback || mutationPending}
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
            activeNavigation="tasks"
            onOverviewOpen={() => undefined}
            onEvidenceOpen={() => setMobileInspectorOpen(true)}
          />
          <main className="repository-empty-workspace">
            <RegressionContractsPanel
              contracts={data.contracts}
              candidates={data.contractCandidates}
              evaluation={data.contractEvaluation}
              disabled={offlineFallback}
              mutationPending={mutationPending}
              onCreateCandidate={handleCreateContractCandidate}
              onUpdateCandidate={handleUpdateContractCandidate}
              onApproveCandidate={handleApproveContractCandidate}
              onSupersedeContract={handleSupersedeContract}
            />
            <EmptyWorkspace repositoryName={data.repository.name} />
          </main>
        </div>
        <BottomNavigation
          activeNavigation="tasks"
          sessionsEnabled={false}
          onTasksOpen={() => undefined}
          onSessionsOpen={() => undefined}
          onEvidenceOpen={() => setMobileInspectorOpen(true)}
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
  const selectedEvidence = selectedCheckpoint
    ? (explicitlySelectedEvidence?.checkpointId === selectedCheckpoint.id
        ? explicitlySelectedEvidence
        : data.evidence.find((evidence) => evidence.checkpointId === selectedCheckpoint.id))
    : undefined;
  const selectedFact = data.facts.find((fact) => fact.id === selectedEvidence?.factId) ?? data.facts[0];
  const resumeCandidate = data.resumeCandidate?.taskId === selectedTask.id ? data.resumeCandidate : undefined;
  const selectedContractEvaluation = data.contractEvaluations.find(
    (evaluation) => evaluation.taskId === selectedTask.id,
  ) ?? (data.contractEvaluation?.taskId === selectedTask.id ? data.contractEvaluation : null);

  const selectTask = (taskId: string) => {
    const task = data.tasks.find((item) => item.id === taskId);
    if (!task) return;
    const checkpointId = task.checkpointIds[0] ?? '';
    setSelectedTaskId(taskId);
    setSelectedCheckpointId(checkpointId);
    setSelectedEvidenceId(data.evidence.find((item) => item.checkpointId === checkpointId)?.id ?? data.evidence[0]?.id ?? '');
    setWorkspaceView('task');
  };

  const openOverview = (focus: 'tasks' | 'sessions') => {
    setOverviewFocus(focus);
    setWorkspaceView('overview');
    setMobileInspectorOpen(false);
  };

  const selectCheckpoint = (checkpoint: Checkpoint) => {
    setSelectedCheckpointId(checkpoint.id);
    const matchingEvidence = data.evidence.find((evidence) => evidence.checkpointId === checkpoint.id);
    if (matchingEvidence) setSelectedEvidenceId(matchingEvidence.id);
    setMobileInspectorOpen(true);
  };

  const selectEvidence = (evidenceId: string) => {
    const evidence = data.evidence.find((item) => item.id === evidenceId);
    if (!evidence) return;
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

  const handleTaskStatus = async (nextStatus: TaskStatus) => {
    if (offlineFallback || mutationPending) return;
    const previousStatus = selectedTask.status;
    setData((current) => current ? {
      ...current,
      tasks: current.tasks.map((task) => task.id === selectedTask.id ? { ...task, status: nextStatus } : task),
    } : current);
    const saved = await performMutation(() => updateTaskStatus(selectedTask.id, nextStatus));
    if (saved === null) {
      setData((current) => current ? {
        ...current,
        tasks: current.tasks.map((task) => task.id === selectedTask.id ? { ...task, status: previousStatus } : task),
      } : current);
    }
  };

  return (
    <div className="app-shell">
      <AppHeader
        repository={data.repository}
        onPreview={() => setContextPackExpanded(true)}
        onExport={() => void handleExport()}
        onPurge={() => void handlePurge()}
        actionsDisabled={offlineFallback || mutationPending}
      />
      {offlineFallback ? <div className="sample-banner" role="status">Local API unavailable · read-only sample workspace · changes are disabled</div> : null}
      {actionError ? <div className="action-error" role="alert">{actionError}</div> : null}
      <div className={`app-body ${workspaceView === 'overview' ? 'overview-app-body' : ''}`}>
        <Sidebar
          query={query}
          status={status}
          tasks={filteredTasks}
          selectedTaskId={selectedTask.id}
          onQueryChange={setQuery}
          onStatusChange={setStatus}
          onTaskSelect={selectTask}
          activeNavigation={workspaceView === 'overview' ? overviewFocus : 'task'}
          onOverviewOpen={openOverview}
          onEvidenceOpen={() => setMobileInspectorOpen(true)}
        />
        {workspaceView === 'overview' ? (
          <ProjectOverview
            tasks={filteredTasks}
            sessions={data.sessions}
            facts={data.facts}
            focus={overviewFocus}
            onTaskSelect={selectTask}
          />
        ) : <TaskWorkspace
          task={selectedTask}
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
          onTaskStatusChange={(nextStatus) => void handleTaskStatus(nextStatus)}
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
            availableEvidence={data.evidence.filter((evidence) => selectedTask.checkpointIds.includes(evidence.checkpointId))}
            fact={selectedFact}
            replacementFacts={data.facts.filter((fact) => fact.id !== selectedFact.id && !['invalid', 'superseded'].includes(fact.status))}
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
        activeNavigation={workspaceView === 'overview' ? overviewFocus : 'tasks'}
        sessionsEnabled={data.sessions.length > 0}
        onTasksOpen={() => openOverview('tasks')}
        onSessionsOpen={() => openOverview('sessions')}
        onEvidenceOpen={() => setMobileInspectorOpen(true)}
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

function EmptyWorkspace({ repositoryName }: { repositoryName: string }) {
  return (
    <section className="repository-empty-state">
      <span className="empty-lineage-mark" aria-hidden="true" />
      <h1>{repositoryName} is connected</h1>
      <p>PreviouslyOn is ready to capture the next Codex session. Start a task in this repository, then return here to review its first verified checkpoint.</p>
      <ol>
        <li><strong>Start Codex</strong><span>Work normally in the connected repository.</span></li>
        <li><strong>Finish the session</strong><span>A local checkpoint is created from captured events and Git state.</span></li>
        <li><strong>Review evidence</strong><span>Confirm facts before they can enter a context pack.</span></li>
      </ol>
    </section>
  );
}

function normalizeBootstrap(bootstrap: BootstrapData): BootstrapData {
  return {
    ...bootstrap,
    contracts: bootstrap.contracts ?? [],
    contractCandidates: bootstrap.contractCandidates ?? [],
    contractEvaluation: bootstrap.contractEvaluation ?? null,
    contractEvaluations: bootstrap.contractEvaluations ?? [],
    sessions: bootstrap.sessions ?? [],
    facts: (bootstrap.facts ?? []).map((fact) => ({
      ...fact,
      taskId: fact.taskId ?? bootstrap.tasks[0]?.id ?? '',
      kind: fact.kind ?? 'note',
      relatedFiles: fact.relatedFiles ?? [],
    })),
    evidence: (bootstrap.evidence ?? []).map((evidence) => ({
      ...evidence,
      sessionId: evidence.sessionId ?? '',
      excludedSession: evidence.excludedSession ?? false,
    })),
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
