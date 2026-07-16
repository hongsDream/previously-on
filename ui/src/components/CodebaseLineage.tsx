import {
  ArrowRight,
  CheckCircle2,
  Files,
  FolderGit2,
  GitBranch,
  GitCommitHorizontal,
  History,
  Target,
} from 'lucide-react';
import type { Task, TemporalStatus } from '../types';

interface CodebaseLineageProps {
  task: Task;
}

const statusLabels: Record<TemporalStatus, string> = {
  unchanged: 'Code unchanged',
  changed: 'Relevant code changed',
  diverged: 'History diverged',
  broken: 'Repository unavailable',
  degraded: 'Validation degraded',
};

export function CodebaseLineage({ task }: CodebaseLineageProps) {
  const { codebase } = task;
  const touchedFiles = task.files.reduce((total, file) => total + file.count, 0);
  const openItems = task.openItems.risks + task.openItems.questions + task.openItems.actions;

  return (
    <section className="codebase-lineage" aria-labelledby="codebase-lineage-title">
      <header className="lineage-header">
        <div>
          <span className="lineage-kicker">Task connection</span>
          <h2 id="codebase-lineage-title">Codebase lineage</h2>
          <p>Where this task ran, what it touched, and how its state was verified.</p>
        </div>
        <span className={`lineage-status lineage-status-${codebase.status}`}>
          {statusLabels[codebase.status]}
        </span>
      </header>

      <div className="codebase-identity">
        <div className="codebase-title">
          <span className="codebase-icon"><FolderGit2 size={19} /></span>
          <div>
            <strong>{codebase.repositoryName}</strong>
            <code title={codebase.worktreeRoot}>{codebase.worktreeRoot}</code>
          </div>
        </div>
        <dl className="codebase-details">
          <div>
            <dt>Repository ID</dt>
            <dd title={task.repositoryId}>{task.repositoryId}</dd>
          </div>
          <div>
            <dt><GitBranch size={12} /> Branch</dt>
            <dd>{codebase.branch}</dd>
          </div>
          <div>
            <dt><GitCommitHorizontal size={12} /> Baseline</dt>
            <dd title={codebase.baselineSha}>{shortSha(codebase.baselineSha)}</dd>
          </div>
          <div>
            <dt><GitCommitHorizontal size={12} /> Current</dt>
            <dd title={codebase.currentSha}>{shortSha(codebase.currentSha)}</dd>
          </div>
        </dl>
        {codebase.registeredRoot !== codebase.worktreeRoot ? (
          <p className="registered-root">
            Registered repository <code>{codebase.registeredRoot}</code>
          </p>
        ) : null}
      </div>

      <div className="lineage-flow" aria-label="Task-centered lineage">
        <LineageNode icon={<History size={17} />} label="Sessions">
          <strong>{codebase.sessionCount} captured</strong>
          <span>{task.checkpointIds.length} verified checkpoints</span>
          <ThreadList threadIds={codebase.sourceThreadIds} />
        </LineageNode>
        <LineageArrow />
        <LineageNode icon={<Target size={17} />} label="Current task">
          <strong>{task.title}</strong>
          <span className="lineage-clamp">{task.goal || 'No task goal captured'}</span>
          <small>{task.decisions.confirmed} decisions · {openItems} open items</small>
        </LineageNode>
        <LineageArrow />
        <LineageNode icon={<Files size={17} />} label="Code areas">
          <strong>{task.files.length} areas · {touchedFiles} touches</strong>
          {task.files.length > 0 ? (
            <ul className="lineage-files">
              {task.files.slice(0, 3).map((file) => (
                <li key={file.path}><code>{file.path}</code><span>{file.count}</span></li>
              ))}
            </ul>
          ) : <span>No file changes captured</span>}
        </LineageNode>
        <LineageArrow />
        <LineageNode icon={<CheckCircle2 size={17} />} label="Verification">
          <strong>{task.tests.passing} passing</strong>
          <span className={task.tests.failing > 0 ? 'lineage-danger' : undefined}>
            {task.tests.failing} failing · {task.tests.skipped} skipped
          </span>
          <small>{statusLabels[codebase.status]}</small>
        </LineageNode>
      </div>
    </section>
  );
}

function LineageNode({ icon, label, children }: { icon: React.ReactNode; label: string; children: React.ReactNode }) {
  return (
    <article className="lineage-node">
      <header>{icon}<span>{label}</span></header>
      <div>{children}</div>
    </article>
  );
}

function LineageArrow() {
  return <ArrowRight className="lineage-arrow" size={16} aria-hidden="true" />;
}

function ThreadList({ threadIds }: { threadIds: string[] }) {
  if (threadIds.length === 0) return <small>No source task IDs captured</small>;
  return (
    <span className="thread-list" title={threadIds.join('\n')}>
      {threadIds.slice(0, 2).map((threadId) => <code key={threadId}>{compactId(threadId)}</code>)}
      {threadIds.length > 2 ? <small>+{threadIds.length - 2}</small> : null}
    </span>
  );
}

function shortSha(sha?: string) {
  return sha ? sha.slice(0, 8) : 'unavailable';
}

function compactId(id: string) {
  return id.length > 14 ? `${id.slice(0, 7)}…${id.slice(-5)}` : id;
}
