import { useDeferredValue, useMemo, useState } from 'react';
import { AppHeader } from './components/AppHeader';
import { AllProjectsView } from './components/AllProjectsView';
import { BottomNavigation } from './components/BottomNavigation';
import { EvidenceInspector } from './components/EvidenceInspector';
import { CodexSyncStatus } from './components/CodexSyncStatus';
import { ErrorNotice } from './components/ErrorNotice';
import { FirstRunSetup, RegisteredEmptyActions } from './components/FirstRunSetup';
import { ProjectOverview } from './components/ProjectOverview';
import { Sidebar } from './components/Sidebar';
import { SettingsPanel } from './components/SettingsPanel';
import { TaskWorkspace } from './components/TaskWorkspace';
import { RegressionContractsPanel } from './components/RegressionContractsPanel';
import { useBootstrap } from './hooks/useBootstrap';
import { useContractActions } from './hooks/useContractActions';
import { useFactActions } from './hooks/useFactActions';
import { useMutationRunner } from './hooks/useMutationRunner';
import { useRefreshActions } from './hooks/useRefreshActions';
import { useTaskActions } from './hooks/useTaskActions';
import { useWorkspaceNavigation } from './hooks/useWorkspaceNavigation';
import { I18nProvider } from './i18n';
import { useI18n } from './i18n-context';
import {
  resolveTaskSelection,
  selectWorkspace,
} from './lib/workspace';
import type {
  AiFactRefreshOperationV1,
  Checkpoint,
  TaskStatus,
} from './types';

export function App() {
  return <I18nProvider><AppContent /></I18nProvider>;
}

function AppContent() {
  const { t } = useI18n();
  const {
    data,
    setData,
    repositories,
    selectedRepositoryId,
    bootstrapRepositoryId,
    allProjects,
    selectRepository,
    showAllProjects,
    syncReport,
    syncPending,
    syncError,
    manualSync,
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

  const isUnregistered = data?.repository.state === 'unregistered';
  const workspace = data ? selectWorkspace(data, selection, filteredTasks) : null;
  const {
    mutationPending,
    setMutationPending,
    actionError,
    setActionError,
    performMutation,
  } = useMutationRunner(offlineFallback || isUnregistered);
  const contractActions = useContractActions({ data, setData, performMutation });
  const factActions = useFactActions({
    selectedFact: workspace?.selectedFact,
    selectedEvidence: workspace?.selectedEvidence,
    offlineFallback,
    mutationPending,
    setData,
    performMutation,
  });
  const taskActions = useTaskActions({
    repositoryId: bootstrapRepositoryId,
    selectedTask: workspace?.selectedTask,
    selection,
    offlineFallback,
    mutationPending,
    installBootstrap,
    setGraphRefreshVersion,
    performMutation,
  });
  const refreshActions = useRefreshActions({
    repositoryId: bootstrapRepositoryId,
    data,
    selectedTask: workspace?.selectedTask,
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
  });

  const projectHeaderProps = {
    repositories,
    selectedRepositoryId,
    allProjects,
    syncPending,
    onRepositorySelect: selectRepository,
    onAllProjects: showAllProjects,
    onSync: manualSync,
  };

  if (fatalError) return <ErrorScreen error={fatalError} />;
  if (allProjects) {
    return (
      <div className="app-shell">
        <AppHeader
          {...projectHeaderProps}
          onPreview={() => undefined}
          onExport={() => undefined}
          onPurge={() => undefined}
          actionsDisabled
          previewDisabled
        />
        <AllProjectsView repositories={repositories} onOpen={selectRepository} />
      </div>
    );
  }
  if (!data) return <LoadingScreen />;
  if (data.tasks.length === 0) {
    return (
      <div className="app-shell">
        <AppHeader
          {...projectHeaderProps}
          repository={data.repository}
          onPreview={() => undefined}
          onExport={() => void refreshActions.exportData()}
          onPurge={() => void refreshActions.purge()}
          actionsDisabled={offlineFallback || isUnregistered || mutationPending}
          previewDisabled
        />
        <CodexSyncStatus report={syncReport} />
        {syncError ? <ErrorNotice error={syncError} /> : null}
        {actionError ? <ErrorNotice error={actionError} /> : null}
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
            {isUnregistered ? (
              <FirstRunSetup
                refreshPending={mutationPending}
                onRefresh={() => void refreshActions.refreshBootstrap()}
              />
            ) : null}
            <RegressionContractsPanel
              contracts={data.contracts}
              candidates={data.contractCandidates}
              evaluation={data.contractEvaluation}
              disabled={offlineFallback || isUnregistered}
              mutationPending={mutationPending}
              onCreateCandidate={contractActions.createCandidate}
              onUpdateCandidate={contractActions.updateCandidate}
              onApproveCandidate={contractActions.approveCandidate}
              onSupersedeContract={contractActions.supersedeContract}
            />
            {isUnregistered ? null : data.repository.state === 'degraded'
              ? <DegradedWorkspace repositoryName={data.repository.name} />
              : <EmptyWorkspace
                  repositoryName={data.repository.name}
                  repositoryPath={data.repository.path}
                  refreshPending={mutationPending}
                  onRefresh={() => void refreshActions.refreshBootstrap()}
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

  const activeWorkspace = workspace!;
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
  } = activeWorkspace;

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


  return (
    <div className="app-shell">
      <AppHeader
        {...projectHeaderProps}
        repository={data.repository}
        onPreview={openContextPack}
        onExport={() => void refreshActions.exportData()}
        onPurge={() => void refreshActions.purge()}
        actionsDisabled={offlineFallback || mutationPending}
        previewDisabled={!selectedCheckpoint || !data.contextPacks[selectedTask.id]}
      />
      <CodexSyncStatus report={syncReport} />
      {syncError ? <ErrorNotice error={syncError} /> : null}
      {offlineFallback ? <div className="sample-banner" role="status">{t('Local API unavailable · read-only sample workspace · changes are disabled')}</div> : null}
      {!offlineFallback && data.repository.state === 'degraded' ? <div className="degraded-banner" role="status">{t('Capture degraded · review missing evidence before trusting this workspace')}</div> : null}
      {actionError ? <ErrorNotice error={actionError} /> : null}
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
          model={{
            task: selectedTask,
            tasks: data.tasks,
            sessions: data.sessions,
            facts: data.facts,
            groupingOperations: data.taskGroupingOperations,
            aiRefreshCapability: data.aiRefreshCapability,
            factRefreshOperation: latestFactRefreshOperation(data.factRefreshOperations, selectedTask.id),
            agents: data.agents,
            checkpoints: taskCheckpoints,
            selectedCheckpoint,
            resumeCandidate,
            contextPack: data.contextPacks[selectedTask.id],
            contracts: data.contracts,
            contractCandidates: data.contractCandidates,
            contractEvaluation: selectedContractEvaluation,
          }}
          actions={{
            onCheckpointSelect: selectCheckpoint,
            onReviewResume: reviewCandidate,
            onDismissResume: dismissCandidate,
            onToggleContextPack: toggleContextPack,
            onTaskUpdate: taskActions.update,
            onGroupingPreview: taskActions.previewGrouping,
            onGroupingApply: taskActions.applyGrouping,
            onGroupingUndo: taskActions.undoGrouping,
            onFactRefreshStart: refreshActions.start,
            onFactRefreshPoll: refreshActions.poll,
            onFactRefreshReview: refreshActions.review,
            onCreateContractCandidate: contractActions.createCandidate,
            onUpdateContractCandidate: contractActions.updateCandidate,
            onApproveContractCandidate: contractActions.approveCandidate,
            onSupersedeContract: contractActions.supersedeContract,
            onBack: () => openOverview('tasks'),
          }}
          uiState={{
            contextPackExpanded,
            contractMutationsDisabled: offlineFallback,
            mutationPending,
          }}
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
            onStatusChange={(nextStatus, supersedesFactId) => void factActions.updateStatus(nextStatus, supersedesFactId)}
            onFactUpdate={factActions.updateContent}
            onSessionExcludedChange={(excluded) => void factActions.setSessionExcluded(excluded)}
            onRevalidate={() => void factActions.revalidate()}
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

function ErrorScreen({ error }: { error: import('./lib/api').UiError }) {
  const { t } = useI18n();
  return (
    <main className="loading-screen" role="alert">
      <span className="loading-mark error-mark" />
      <h1>{t('PreviouslyOn could not load')}</h1>
      <p>{t(error.messageKey)}</p>
      {error.technicalDetails.length > 0 ? (
        <details>
          <summary>{t('Technical details')}</summary>
          <ul>{error.technicalDetails.map((detail) => <li key={detail}>{detail}</li>)}</ul>
        </details>
      ) : null}
    </main>
  );
}

function LoadingScreen() {
  const { t } = useI18n();
  return (
    <main className="loading-screen" aria-busy="true">
      <span className="loading-mark" />
      <p>{t('Loading PreviouslyOn…')}</p>
    </main>
  );
}

function EmptyWorkspace({ repositoryName, repositoryPath, refreshPending, onRefresh }: {
  repositoryName: string;
  repositoryPath: string;
  refreshPending: boolean;
  onRefresh: () => void;
}) {
  const { t } = useI18n();
  return (
    <section className="repository-empty-state">
      <span className="empty-lineage-mark" aria-hidden="true" />
      <h1>{t('{name} is connected', { name: repositoryName })}</h1>
      <p>{t('Start one captured Codex session, finish it normally, then refresh this screen to review the first checkpoint and its evidence.')}</p>
      <RegisteredEmptyActions repositoryPath={repositoryPath} refreshPending={refreshPending} onRefresh={onRefresh} />
    </section>
  );
}

function DegradedWorkspace({ repositoryName }: { repositoryName: string }) {
  const { t } = useI18n();
  return (
    <section className="repository-empty-state degraded-empty-state">
      <span className="empty-lineage-mark" aria-hidden="true" />
      <h1>{t('{name} capture is degraded', { name: repositoryName })}</h1>
      <p>{t('PreviouslyOn found the registered repository, but it cannot confirm a complete first checkpoint. Run previously doctor, then start a new captured session after resolving the reported issue.')}</p>
    </section>
  );
}

function latestFactRefreshOperation(operations: AiFactRefreshOperationV1[], taskId: string) {
  return operations
    .filter((operation) => operation.taskId === taskId)
    .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt))[0];
}
