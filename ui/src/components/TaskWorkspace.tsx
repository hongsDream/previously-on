import { ArrowLeft, MoreHorizontal } from 'lucide-react';
import type { BootstrapData, Checkpoint, RegressionCandidateDraftV1, Task, TaskStatus } from '../types';
import { CheckpointTimeline } from './CheckpointTimeline';
import { ContextPackPreview } from './ContextPackPreview';
import { ResumeBanner } from './ResumeBanner';
import { RegressionContractsPanel } from './RegressionContractsPanel';

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
  onCheckpointSelect: (checkpoint: Checkpoint) => void;
  onReviewResume: () => void;
  onDismissResume: () => void;
  onToggleContextPack: () => void;
  onTaskStatusChange: (status: TaskStatus) => void;
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
  onCheckpointSelect,
  onReviewResume,
  onDismissResume,
  onToggleContextPack,
  onTaskStatusChange,
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
        <button className="back-button" type="button" onClick={onBack}><ArrowLeft size={18} /> <span className="desktop-only">All tasks</span></button>
        <div>
          <h1>{task.title}</h1>
          <span className="task-meta desktop-only">
            <small>Task ID: &nbsp;{task.id}</small>
            <select aria-label="Task status" value={task.status} disabled={mutationPending} onChange={(event) => onTaskStatusChange(event.target.value as TaskStatus)}>
              <option value="active">Active</option>
              <option value="completed">Completed</option>
              <option value="abandoned">Abandoned</option>
            </select>
          </span>
        </div>
        <button className="icon-button mobile-only" type="button" aria-label="Task options" disabled><MoreHorizontal size={21} /></button>
      </header>

      {resumeCandidate ? (
        <ResumeBanner candidate={resumeCandidate} task={task} onReview={onReviewResume} onDismiss={onDismissResume} />
      ) : null}

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

function EmptyTask() {
  return (
    <section className="empty-task">
      <h2>No checkpoints yet</h2>
      <p>New verified sessions for this task will appear here.</p>
    </section>
  );
}
