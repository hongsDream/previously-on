import { useEffect, useMemo, useState } from 'react';
import { ArrowRight, History, Merge, Move, Scissors, Undo2, X } from 'lucide-react';
import type {
  Fact,
  Session,
  Task,
  TaskGroupingAction,
  TaskGroupingOperationV1,
  TaskGroupingPreviewV1,
  TaskGroupingRequestV1,
} from '../types';

type GroupingAction = Exclude<TaskGroupingAction, 'undo'>;

interface TaskGroupingPanelProps {
  task: Task;
  tasks: Task[];
  sessions: Session[];
  facts: Fact[];
  operations: TaskGroupingOperationV1[];
  disabled: boolean;
  mutationPending: boolean;
  onPreview: (request: TaskGroupingRequestV1) => Promise<TaskGroupingPreviewV1 | null>;
  onApply: (request: TaskGroupingRequestV1) => Promise<boolean>;
  onUndo: (operationId: string) => Promise<boolean>;
}

export function TaskGroupingPanel({ task, tasks, sessions, facts, operations, disabled, mutationPending, onPreview, onApply, onUndo }: TaskGroupingPanelProps) {
  const taskSessions = useMemo(() => sessions.filter((session) => session.taskId === task.id), [sessions, task.id]);
  const targetTasks = useMemo(
    () => tasks.filter((candidate) => candidate.id !== task.id && candidate.repositoryId === task.repositoryId),
    [tasks, task.id, task.repositoryId],
  );
  const history = useMemo(() => operations.filter((operation) => operationTouchesTask(operation, task.id)), [operations, task.id]);
  const [open, setOpen] = useState(false);
  const [action, setAction] = useState<GroupingAction>('move');
  const [operationId, setOperationId] = useState(newOperationId);
  const [selectedSessionIds, setSelectedSessionIds] = useState<string[]>([]);
  const [targetTaskId, setTargetTaskId] = useState('');
  const [newTaskTitle, setNewTaskTitle] = useState('');
  const [newTaskGoal, setNewTaskGoal] = useState('');
  const [preview, setPreview] = useState<TaskGroupingPreviewV1 | null>(null);
  const [previewRequest, setPreviewRequest] = useState<TaskGroupingRequestV1 | null>(null);
  const [validationError, setValidationError] = useState('');

  useEffect(() => {
    setOpen(false);
    resetDraft();
  }, [task.id]);

  const begin = () => {
    resetDraft();
    setTargetTaskId(targetTasks[0]?.id ?? '');
    setOpen(true);
  };

  const changeAction = (nextAction: GroupingAction) => {
    setAction(nextAction);
    setSelectedSessionIds(nextAction === 'merge' ? taskSessions.map((session) => session.id) : []);
    invalidatePreview();
  };

  const toggleSession = (sessionId: string) => {
    setSelectedSessionIds((current) => current.includes(sessionId)
      ? current.filter((id) => id !== sessionId)
      : [...current, sessionId]);
    invalidatePreview();
  };

  const buildRequest = (): TaskGroupingRequestV1 | null => {
    if (selectedSessionIds.length === 0) {
      setValidationError('Select at least one session.');
      return null;
    }
    if ((action === 'move' || action === 'merge') && !targetTaskId) {
      setValidationError('Select a target task.');
      return null;
    }
    if (action === 'split' && !newTaskTitle.trim()) {
      setValidationError('A new task title is required for split.');
      return null;
    }
    setValidationError('');
    return {
      operationId,
      action,
      sessionIds: [...selectedSessionIds].sort(),
      fromTaskId: task.id,
      ...(action === 'split'
        ? { newTaskTitle: newTaskTitle.trim(), newTaskGoal: newTaskGoal.trim() }
        : { targetTaskId }),
    };
  };

  const requestPreview = async () => {
    const request = buildRequest();
    if (!request) return;
    const result = await onPreview(request);
    if (!result) return;
    setPreview(result);
    setPreviewRequest(request);
  };

  const confirm = async () => {
    if (!previewRequest || !preview) return;
    if (await onApply(previewRequest)) {
      setOpen(false);
      resetDraft();
    }
  };

  return (
    <section className="task-grouping-panel" aria-labelledby="task-grouping-title">
      <header>
        <div>
          <span className="task-integrity-kicker">Append-only organization</span>
          <h2 id="task-grouping-title">Session grouping</h2>
          <p>Preview session and fact impact before moving task history.</p>
        </div>
        <button className="secondary-button" type="button" disabled={disabled || mutationPending || open || taskSessions.length === 0} onClick={begin}>
          <Move size={14} /> Organize sessions
        </button>
      </header>

      {open ? (
        <div className="task-grouping-editor">
          <div className="task-editor-heading">
            <strong>Preview a grouping operation</strong>
            <button className="icon-button" type="button" aria-label="Close grouping editor" onClick={() => setOpen(false)}><X size={16} /></button>
          </div>
          {validationError ? <p className="task-editor-error" role="alert">{validationError}</p> : null}
          <fieldset disabled={disabled || mutationPending}>
            <legend>Action</legend>
            <div className="grouping-action-picker">
              <ActionButton action="move" current={action} icon={<Move size={14} />} onSelect={changeAction}>Move</ActionButton>
              <ActionButton action="merge" current={action} icon={<Merge size={14} />} onSelect={changeAction}>Merge</ActionButton>
              <ActionButton action="split" current={action} icon={<Scissors size={14} />} onSelect={changeAction}>Split</ActionButton>
            </div>
          </fieldset>

          <fieldset disabled={disabled || mutationPending}>
            <legend>Sessions from {task.title}</legend>
            <div className="grouping-session-list">
              {taskSessions.map((session) => (
                <label key={session.id}>
                  <input
                    type="checkbox"
                    checked={selectedSessionIds.includes(session.id)}
                    disabled={action === 'merge'}
                    onChange={() => toggleSession(session.id)}
                  />
                  <span><strong>{session.sourceThreadId ?? session.id}</strong><small>{session.id} · {session.turnCount} turns · {session.compactionCount} compactions</small></span>
                </label>
              ))}
            </div>
          </fieldset>

          {action === 'split' ? (
            <fieldset className="grouping-target-fields" disabled={disabled || mutationPending}>
              <legend>New active task</legend>
              <label>Title<input value={newTaskTitle} onChange={(event) => { setNewTaskTitle(event.target.value); invalidatePreview(); }} /></label>
              <label>Goal<textarea rows={3} value={newTaskGoal} onChange={(event) => { setNewTaskGoal(event.target.value); invalidatePreview(); }} /></label>
            </fieldset>
          ) : (
            <fieldset className="grouping-target-fields" disabled={disabled || mutationPending}>
              <legend>Target task</legend>
              <label>Task
                <select value={targetTaskId} onChange={(event) => { setTargetTaskId(event.target.value); invalidatePreview(); }}>
                  <option value="">Select a task</option>
                  {targetTasks.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.title} · {candidate.status}</option>)}
                </select>
              </label>
            </fieldset>
          )}

          <div className="grouping-editor-actions">
            <code title={operationId}>Operation {compactId(operationId)}</code>
            <button className="secondary-button" type="button" disabled={disabled || mutationPending} onClick={() => void requestPreview()}>Preview impact</button>
          </div>

          {preview ? (
            <GroupingPreview preview={preview} facts={facts} action={action} disabled={disabled || mutationPending} onConfirm={() => void confirm()} />
          ) : null}
        </div>
      ) : null}

      <OperationHistory history={history} operations={operations} disabled={disabled || mutationPending} onUndo={onUndo} />
    </section>
  );

  function invalidatePreview() {
    setPreview(null);
    setPreviewRequest(null);
    setOperationId(newOperationId());
    setValidationError('');
  }

  function resetDraft() {
    setAction('move');
    setOperationId(newOperationId());
    setSelectedSessionIds([]);
    setTargetTaskId('');
    setNewTaskTitle('');
    setNewTaskGoal('');
    setPreview(null);
    setPreviewRequest(null);
    setValidationError('');
  }
}

function ActionButton({ action, current, icon, children, onSelect }: {
  action: GroupingAction;
  current: GroupingAction;
  icon: React.ReactNode;
  children: React.ReactNode;
  onSelect: (action: GroupingAction) => void;
}) {
  return <button className={action === current ? 'active' : ''} type="button" aria-pressed={action === current} onClick={() => onSelect(action)}>{icon}{children}</button>;
}

function GroupingPreview({ preview, facts, action, disabled, onConfirm }: {
  preview: TaskGroupingPreviewV1;
  facts: Fact[];
  action: GroupingAction;
  disabled: boolean;
  onConfirm: () => void;
}) {
  const sessionMoves = preview.affectedSessions.length ? preview.affectedSessions : preview.operation.sessionMoves;
  const factImpacts = preview.affectedFacts.length ? preview.affectedFacts : preview.operation.factImpacts;
  return (
    <section className="grouping-preview" aria-labelledby="grouping-preview-title" aria-live="polite">
      <header>
        <div><h3 id="grouping-preview-title">Impact preview</h3><small>{preview.counts.sessions} sessions · {preview.counts.factsMoved} moved facts · {preview.counts.factsMixed} mixed facts</small></div>
        <button className="primary-button" type="button" disabled={disabled} onClick={onConfirm}>Confirm {action}</button>
      </header>
      <div className="grouping-preview-grid">
        <section>
          <h3>Affected sessions</h3>
          <ul>{sessionMoves.map((move) => <li key={move.sessionId}><code>{move.sessionId}</code><span>{compactId(move.fromTaskId)} <ArrowRight size={12} aria-hidden="true" /> {compactId(move.toTaskId)}</span></li>)}</ul>
        </section>
        <section>
          <h3>Affected facts</h3>
          {factImpacts.length ? <ul>{factImpacts.map((impact) => {
            const fact = facts.find((candidate) => candidate.id === impact.factId);
            return <li key={impact.factId} className={impact.mixedProvenance ? 'mixed-impact' : ''}><strong>{fact?.text ?? impact.factId}</strong><small>{impact.mixedProvenance ? 'Retained in the source task · mixed provenance · not duplicated' : `Moves to ${compactId(impact.toTaskId ?? '')}`}</small></li>;
          })}</ul> : <p>No facts change task association.</p>}
        </section>
      </div>
    </section>
  );
}

function OperationHistory({ history, operations, disabled, onUndo }: {
  history: TaskGroupingOperationV1[];
  operations: TaskGroupingOperationV1[];
  disabled: boolean;
  onUndo: (operationId: string) => Promise<boolean>;
}) {
  return (
    <section className="grouping-history" aria-labelledby="grouping-history-title">
      <header><span><History size={14} /><strong id="grouping-history-title">Operation history</strong></span><small>{history.length}</small></header>
      {history.length ? (
        <ol>{history.map((operation) => {
          const inverse = operations.find((candidate) => candidate.inverseOf === operation.operationId);
          const canUndo = operation.action !== 'undo' && !inverse;
          return (
            <li key={operation.operationId}>
              <span><strong>{operation.action}</strong><code title={operation.operationId}>{compactId(operation.operationId)}</code><small>{formatOccurredAt(operation.occurredAt)} · {operation.sessionMoves.length} sessions</small></span>
              {operation.inverseOf ? <small>Inverse of {compactId(operation.inverseOf)}</small> : null}
              <button className="secondary-button" type="button" disabled={disabled || !canUndo} aria-label={`Undo grouping operation ${operation.operationId}`} onClick={() => void onUndo(operation.operationId)}><Undo2 size={13} /> {inverse ? 'Undone' : 'Undo'}</button>
            </li>
          );
        })}</ol>
      ) : <p>No grouping operations recorded.</p>}
    </section>
  );
}

function operationTouchesTask(operation: TaskGroupingOperationV1, taskId: string) {
  return operation.sessionMoves.some((move) => move.fromTaskId === taskId || move.toTaskId === taskId)
    || operation.taskLifecycle.some((snapshot) => snapshot.taskId === taskId)
    || operation.createdTask?.id === taskId;
}

function newOperationId() {
  return globalThis.crypto?.randomUUID?.() ?? `grouping-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function compactId(value: string) {
  if (!value) return 'unavailable';
  return value.length > 18 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
}

function formatOccurredAt(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? 'time unavailable' : date.toLocaleString();
}
