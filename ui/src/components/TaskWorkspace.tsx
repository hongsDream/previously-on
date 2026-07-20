import { ArrowLeft, CircleAlert, CircleCheck, LoaderCircle } from 'lucide-react';
import type {
  AiFactRefreshOperationV1,
  AiRefreshCapabilityV1,
  AgentV1,
  BootstrapData,
  Checkpoint,
  Fact,
  FactKind,
  RegressionCandidateDraftV1,
  Session,
  Task,
  TaskGroupingOperationV1,
  TaskGroupingPreviewV1,
  TaskGroupingRequestV1,
  TaskUpdateV1,
} from '../types';
import type { FactCandidateReviewResponse } from '../lib/api';
import { AgentsTree } from './AgentsTree';
import { CheckpointTimeline } from './CheckpointTimeline';
import { CodebaseLineage } from './CodebaseLineage';
import { ContextPackPreview } from './ContextPackPreview';
import { FactRefreshPanel } from './FactRefreshPanel';
import { RegressionContractsPanel } from './RegressionContractsPanel';
import { ResumeBanner } from './ResumeBanner';
import { TaskEditor } from './TaskEditor';
import { TaskGroupingPanel } from './TaskGroupingPanel';

interface TaskWorkspaceModel {
  task: Task;
  checkpoints: Checkpoint[];
  selectedCheckpoint?: Checkpoint;
  resumeCandidate?: BootstrapData['resumeCandidate'];
  contextPack?: BootstrapData['contextPacks'][string];
  contracts: BootstrapData['contracts'];
  contractCandidates: BootstrapData['contractCandidates'];
  contractEvaluation: BootstrapData['contractEvaluation'];
  tasks: Task[];
  sessions: Session[];
  facts: Fact[];
  groupingOperations: TaskGroupingOperationV1[];
  aiRefreshCapability: AiRefreshCapabilityV1;
  factRefreshOperation?: AiFactRefreshOperationV1;
  agents: AgentV1[];
}

interface TaskWorkspaceActions {
  onCheckpointSelect: (checkpoint: Checkpoint) => void;
  onReviewResume: () => void;
  onDismissResume: () => void;
  onToggleContextPack: () => void;
  onTaskUpdate: (update: TaskUpdateV1) => Promise<boolean>;
  onGroupingPreview: (request: TaskGroupingRequestV1) => Promise<TaskGroupingPreviewV1 | null>;
  onGroupingApply: (request: TaskGroupingRequestV1) => Promise<boolean>;
  onGroupingUndo: (operationId: string) => Promise<boolean>;
  onFactRefreshStart: (requestId: string) => Promise<AiFactRefreshOperationV1 | null>;
  onFactRefreshPoll: (operationId: string, signal: AbortSignal) => Promise<AiFactRefreshOperationV1 | null>;
  onFactRefreshReview: (operationId: string, candidateId: string, decision: 'accept' | 'reject', content?: string, kind?: FactKind) => Promise<FactCandidateReviewResponse | null>;
  onCreateContractCandidate: (candidate: RegressionCandidateDraftV1) => Promise<boolean>;
  onUpdateContractCandidate: (id: string, candidate: RegressionCandidateDraftV1) => Promise<boolean>;
  onApproveContractCandidate: (id: string) => Promise<boolean>;
  onSupersedeContract: (id: string, supersededBy: string) => Promise<boolean>;
  onBack: () => void;
}

interface TaskWorkspaceUiState {
  contextPackExpanded: boolean;
  contractMutationsDisabled: boolean;
  mutationPending: boolean;
}

interface TaskWorkspaceProps {
  model: TaskWorkspaceModel;
  actions: TaskWorkspaceActions;
  uiState: TaskWorkspaceUiState;
}

export function TaskWorkspace({ model, actions, uiState }: TaskWorkspaceProps) {
  const {
    task,
    checkpoints,
    selectedCheckpoint,
    resumeCandidate,
    contextPack,
    contracts,
    contractCandidates,
    contractEvaluation,
    tasks,
    sessions,
    facts,
    groupingOperations,
    aiRefreshCapability,
    factRefreshOperation,
    agents,
  } = model;
  const {
    onCheckpointSelect,
    onReviewResume,
    onDismissResume,
    onToggleContextPack,
    onTaskUpdate,
    onGroupingPreview,
    onGroupingApply,
    onGroupingUndo,
    onFactRefreshStart,
    onFactRefreshPoll,
    onFactRefreshReview,
    onCreateContractCandidate,
    onUpdateContractCandidate,
    onApproveContractCandidate,
    onSupersedeContract,
    onBack,
  } = actions;
  const { contextPackExpanded, contractMutationsDisabled, mutationPending } = uiState;
  return (
    <main className="task-workspace">
      <header className="task-header">
        <button className="back-button" type="button" aria-label="All tasks" onClick={onBack}><ArrowLeft size={18} /> <span className="desktop-only">All tasks</span></button>
        <div>
          <h1>{task.title}</h1>
          <span className="task-meta desktop-only">
            <small>Task ID: &nbsp;{task.id}</small>
            <span className={`task-lifecycle task-lifecycle-${task.status}`}>{task.status}</span>
          </span>
        </div>
      </header>

      <TaskEditor task={task} disabled={contractMutationsDisabled} mutationPending={mutationPending} onSave={onTaskUpdate} />

      <TaskGroupingPanel
        task={task}
        tasks={tasks}
        sessions={sessions}
        facts={facts}
        operations={groupingOperations}
        disabled={contractMutationsDisabled}
        mutationPending={mutationPending}
        onPreview={onGroupingPreview}
        onApply={onGroupingApply}
        onUndo={onGroupingUndo}
      />

      <FactRefreshPanel
        task={task}
        capability={aiRefreshCapability}
        initialOperation={factRefreshOperation}
        disabled={contractMutationsDisabled}
        mutationPending={mutationPending}
        onStart={onFactRefreshStart}
        onPoll={onFactRefreshPoll}
        onReview={onFactRefreshReview}
      />

      {resumeCandidate ? (
        <ResumeBanner candidate={resumeCandidate} task={task} onReview={onReviewResume} onDismiss={onDismissResume} />
      ) : null}

      {task.rollover ? <AutomaticRolloverBanner task={task} /> : null}

      <CodebaseLineage task={task} />

      <AgentsTree task={task} agents={agents} />

      <RegressionContractsPanel
        contracts={contracts}
        candidates={contractCandidates}
        evaluation={contractEvaluation}
        disabled={contractMutationsDisabled}
        mutationPending={mutationPending}
        onCreateCandidate={onCreateContractCandidate}
        onUpdateCandidate={onUpdateContractCandidate}
        onApproveCandidate={onApproveContractCandidate}
        onSupersedeContract={onSupersedeContract}
      />

      {checkpoints.length > 0 && selectedCheckpoint ? (
        <>
          <CheckpointTimeline checkpoints={checkpoints} selectedId={selectedCheckpoint.id} onSelect={onCheckpointSelect} />
          {contextPack ? (
            <ContextPackPreview
              checkpoint={selectedCheckpoint}
              contextPack={contextPack}
              expanded={contextPackExpanded}
              onToggle={onToggleContextPack}
            />
          ) : null}
        </>
      ) : <EmptyTask />}
    </main>
  );
}

function AutomaticRolloverBanner({ task }: { task: Task }) {
  const rollover = task.rollover!;
  const Icon = rollover.status === 'started' ? CircleCheck : rollover.status === 'failed' ? CircleAlert : LoaderCircle;
  const title = rollover.status === 'started'
    ? 'Continued in a fresh Codex task'
    : rollover.status === 'failed'
      ? 'Continuation did not start'
      : 'Fresh Codex task is being prepared';
  return (
    <section className={`automatic-rollover-banner rollover-${rollover.status}`} aria-label="Continuation status">
      <Icon size={19} className={rollover.status === 'pending' || rollover.status === 'thread_created' ? 'spin-icon' : ''} />
      <span>
        <strong>{title}</strong>
        <small>{rollover.message ?? (rollover.status === 'failed' ? 'The original request was left in this task so work can continue safely.' : 'The verified Context Pack and current request were started only after approval.')}</small>
      </span>
      {rollover.newThreadId ? (
        <span className="rollover-actions">
          <code title={rollover.newThreadId}>Task {shortId(rollover.newThreadId)}</code>
          <a className="secondary-button" href={codexThreadUrl(rollover.newThreadId)}>Open in Codex</a>
        </span>
      ) : null}
    </section>
  );
}

function shortId(value: string) {
  return value.length > 16 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
}

function codexThreadUrl(threadId: string) {
  return `codex://threads/${encodeURIComponent(threadId)}`;
}

function EmptyTask() {
  return (
    <section className="empty-task">
      <h2>No checkpoints yet</h2>
      <p>New verified sessions for this task will appear here.</p>
    </section>
  );
}
