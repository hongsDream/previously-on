import { ArrowLeft, CircleAlert, CircleCheck, LoaderCircle } from 'lucide-react';
import type {
  BootstrapData,
  Checkpoint,
  Fact,
  RegressionCandidateDraftV1,
  Session,
  Task,
  TaskGroupingOperationV1,
  TaskGroupingPreviewV1,
  TaskGroupingRequestV1,
  TaskUpdateV1,
} from '../types';
import { CheckpointTimeline } from './CheckpointTimeline';
import { CodebaseLineage } from './CodebaseLineage';
import { ContextPackPreview } from './ContextPackPreview';
import { ResumeBanner } from './ResumeBanner';
import { RegressionContractsPanel } from './RegressionContractsPanel';
import { TaskEditor } from './TaskEditor';
import { TaskGroupingPanel } from './TaskGroupingPanel';

interface TaskWorkspaceProps {
  task: Task;
  checkpoints: Checkpoint[];
  selectedCheckpoint?: Checkpoint;
  resumeCandidate?: BootstrapData['resumeCandidate'];
  contextPack?: BootstrapData['contextPacks'][string];
  contracts: BootstrapData['contracts'];
  contractCandidates: BootstrapData['contractCandidates'];
  contractEvaluation: BootstrapData['contractEvaluation'];
  contextPackExpanded: boolean;
  tasks: Task[];
  sessions: Session[];
  facts: Fact[];
  groupingOperations: TaskGroupingOperationV1[];
  onCheckpointSelect: (checkpoint: Checkpoint) => void;
  onReviewResume: () => void;
  onDismissResume: () => void;
  onToggleContextPack: () => void;
  onTaskUpdate: (update: TaskUpdateV1) => Promise<boolean>;
  onGroupingPreview: (request: TaskGroupingRequestV1) => Promise<TaskGroupingPreviewV1 | null>;
  onGroupingApply: (request: TaskGroupingRequestV1) => Promise<boolean>;
  onGroupingUndo: (operationId: string) => Promise<boolean>;
  onCreateContractCandidate: (candidate: RegressionCandidateDraftV1) => Promise<boolean>;
  onUpdateContractCandidate: (id: string, candidate: RegressionCandidateDraftV1) => Promise<boolean>;
  onApproveContractCandidate: (id: string) => Promise<boolean>;
  onSupersedeContract: (id: string, supersededBy: string) => Promise<boolean>;
  contractMutationsDisabled: boolean;
  mutationPending: boolean;
  onBack: () => void;
}

export function TaskWorkspace({
  task,
  checkpoints,
  selectedCheckpoint,
  resumeCandidate,
  contextPack,
  contracts,
  contractCandidates,
  contractEvaluation,
  contextPackExpanded,
  tasks,
  sessions,
  facts,
  groupingOperations,
  onCheckpointSelect,
  onReviewResume,
  onDismissResume,
  onToggleContextPack,
  onTaskUpdate,
  onGroupingPreview,
  onGroupingApply,
  onGroupingUndo,
  onCreateContractCandidate,
  onUpdateContractCandidate,
  onApproveContractCandidate,
  onSupersedeContract,
  contractMutationsDisabled,
  mutationPending,
  onBack,
}: TaskWorkspaceProps) {
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

      {resumeCandidate ? (
        <ResumeBanner candidate={resumeCandidate} task={task} onReview={onReviewResume} onDismiss={onDismissResume} />
      ) : null}

      {task.rollover ? <AutomaticRolloverBanner task={task} /> : null}

      <CodebaseLineage task={task} />

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
      ? 'Automatic continuation did not start'
      : 'Fresh Codex task is being prepared';
  return (
    <section className={`automatic-rollover-banner rollover-${rollover.status}`} aria-label="Automatic continuation status">
      <Icon size={19} className={rollover.status === 'pending' || rollover.status === 'thread_created' ? 'spin-icon' : ''} />
      <span>
        <strong>{title}</strong>
        <small>{rollover.message ?? (rollover.status === 'failed' ? 'The original request was left in this task so work can continue safely.' : 'The source prompt was blocked only after the new turn started.')}</small>
      </span>
      {rollover.newThreadId ? <code title={rollover.newThreadId}>Task {shortId(rollover.newThreadId)}</code> : null}
    </section>
  );
}

function shortId(value: string) {
  return value.length > 16 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
}

function EmptyTask() {
  return (
    <section className="empty-task">
      <h2>No checkpoints yet</h2>
      <p>New verified sessions for this task will appear here.</p>
    </section>
  );
}
