import { useEffect, useMemo, useRef } from 'react';
import { AlertCircle, ArrowRight, CheckCircle2, Clock3, Code2, GitBranch, ListTodo, MessageSquareText } from 'lucide-react';
import type { Fact, RelationshipGraphSummaryV1, Session, Task } from '../types';
import { useI18n } from '../i18n-context';
import { RelationshipGraphPanel } from './RelationshipGraphPanel';

interface ProjectOverviewProps {
  tasks: Task[];
  graphTasks: Task[];
  sessions: Session[];
  facts: Fact[];
  repositoryId: string;
  graphSummary: RelationshipGraphSummaryV1;
  graphRefreshVersion: number;
  graphDisabled: boolean;
  focus: 'tasks' | 'sessions';
  onTaskSelect: (taskId: string) => void;
}

export function ProjectOverview({ tasks, graphTasks, sessions, facts, repositoryId, graphSummary, graphRefreshVersion, graphDisabled, focus, onTaskSelect }: ProjectOverviewProps) {
  const { locale, t } = useI18n();
  const dateFormatter = useMemo(() => new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }), [locale]);
  const overviewRoot = useRef<HTMLElement>(null);
  const sessionSection = useRef<HTMLElement>(null);
  const activeTasks = tasks.filter((task) => task.status === 'active');
  const recentSessions = [...sessions]
    .sort((left, right) => Date.parse(right.lastActivityAt ?? right.startedAt) - Date.parse(left.lastActivityAt ?? left.startedAt))
    .slice(0, 8);
  const decisions = facts.filter((fact) => fact.kind === 'decision' && !['invalid', 'superseded'].includes(fact.status));
  const openItems = facts.filter((fact) => fact.kind === 'open_item' && !['invalid', 'superseded'].includes(fact.status));
  const tasksById = useMemo(() => new Map(tasks.map((task) => [task.id, task])), [tasks]);

  useEffect(() => {
    const target = focus === 'sessions' ? sessionSection.current : overviewRoot.current;
    target?.scrollIntoView?.({ block: 'start' });
  }, [focus]);

  return (
    <main ref={overviewRoot} className="project-overview" aria-label={t('Project overview')}>
      <header className="overview-hero">
        <div>
          <span>{t('Project memory')}</span>
          <h1>{t('What this codebase remembers')}</h1>
          <p>{t('Tasks, Codex sessions, decisions, open work, and the files they came from—without treating captured history as executable instructions.')}</p>
        </div>
        <dl>
          <div><dt>{t('Active tasks')}</dt><dd>{activeTasks.length}</dd></div>
          <div><dt>{t('Captured sessions')}</dt><dd>{sessions.length}</dd></div>
          <div><dt>{t('Verified decisions')}</dt><dd>{decisions.filter((fact) => ['confirmed', 'pinned'].includes(fact.status)).length}</dd></div>
        </dl>
      </header>

      <section id="overview-tasks" className={`overview-panel overview-tasks ${focus === 'tasks' ? 'overview-focus' : ''}`}>
        <header><span><ListTodo size={17} /><strong>{t('Active tasks')}</strong></span><small>{t('{count} active', { count: activeTasks.length })}</small></header>
        <div className="overview-task-grid">
          {activeTasks.length ? activeTasks.map((task) => (
            <button key={task.id} type="button" onClick={() => onTaskSelect(task.id)}>
              <span className="overview-card-heading"><strong>{task.title}</strong><ArrowRight size={15} /></span>
              <p>{task.goal || t('No goal captured yet.')}</p>
              <span className="overview-codebase"><Code2 size={13} /> {task.codebase.repositoryName}<GitBranch size={12} /> {task.codebase.branch}</span>
              <span className="overview-card-footer">
                <small>{t('{checkpoints} checkpoints · {sessions} sessions', { checkpoints: task.checkpointIds.length, sessions: task.codebase.sessionCount })}</small>
                {task.rollover?.status ? <em className={`rollover-pill rollover-${task.rollover.status}`}>{t(rolloverLabel(task.rollover.status))}</em> : null}
              </span>
            </button>
          )) : <EmptyCopy text={t('No active tasks. Completed work remains available in the task list.')} />}
        </div>
      </section>

      <section ref={sessionSection} id="overview-sessions" className={`overview-panel overview-sessions ${focus === 'sessions' ? 'overview-focus' : ''}`}>
        <header><span><Clock3 size={17} /><strong>{t('Recent sessions')}</strong></span><small>{t('Newest first')}</small></header>
        {recentSessions.length ? (
          <ol>
            {recentSessions.map((session) => {
              const task = tasksById.get(session.taskId);
              const usage = session.contextUsage && session.contextUsage.modelContextWindow > 0
                ? Math.round((session.contextUsage.totalTokens / session.contextUsage.modelContextWindow) * 100)
                : null;
              return (
                <li key={session.id} className={session.excluded ? 'session-excluded' : ''}>
                  <button type="button" onClick={() => task && onTaskSelect(task.id)} disabled={!task}>
                    <span><strong>{task?.title ?? session.taskId}</strong><small>{dateFormatter.format(new Date(session.lastActivityAt ?? session.startedAt))}</small></span>
                    <span><code>{session.sourceThreadId ?? t('No source task ID')}</code><small>{usage === null ? t('{count} compactions · usage unavailable', { count: session.compactionCount }) : t('{count} compactions · {usage}% context', { count: session.compactionCount, usage })}</small></span>
                    <span className={`session-state-label state-${session.continuationState}`}>{session.excluded ? t('Excluded from memory') : session.continuationState}</span>
                  </button>
                </li>
              );
            })}
          </ol>
        ) : <EmptyCopy text={t('No Codex sessions have been captured for this project yet.')} />}
      </section>

      <div className="overview-column-grid">
        <FactSummary title={t('Decisions')} icon={<CheckCircle2 size={17} />} facts={decisions} empty={t('No active decisions captured.')} />
        <FactSummary title={t('Open items')} icon={<AlertCircle size={17} />} facts={openItems} empty={t('No unresolved items captured.')} />
      </div>

      <RelationshipGraphPanel
        repositoryId={repositoryId}
        tasks={graphTasks}
        summary={graphSummary}
        refreshVersion={graphRefreshVersion}
        disabled={graphDisabled}
      />
    </main>
  );
}

function FactSummary({ title, icon, facts, empty }: { title: string; icon: React.ReactNode; facts: Fact[]; empty: string }) {
  const { t } = useI18n();
  return (
    <section className="overview-panel overview-facts">
      <header><span>{icon}<strong>{title}</strong></span><small>{facts.length}</small></header>
      {facts.length ? (
        <ul>{facts.slice(0, 6).map((fact) => (
          <li key={fact.id}>
            <MessageSquareText size={14} />
            <span><strong>{fact.text}</strong><small>{fact.status} · {fact.selectionReason ?? t('Not selected in the current Context Pack')}</small></span>
          </li>
        ))}</ul>
      ) : <EmptyCopy text={empty} />}
    </section>
  );
}

function EmptyCopy({ text }: { text: string }) {
  return <p className="overview-empty">{text}</p>;
}

function rolloverLabel(status: Task['rollover'] extends infer T ? T extends { status: infer S } ? S : never : never) {
  switch (status) {
    case 'started': return 'Continued in fresh task';
    case 'failed': return 'Rollover failed';
    case 'thread_created': return 'Fresh task recovering';
    default: return 'Rollover pending';
  }
}
