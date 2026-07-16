import { useEffect, useState } from 'react';
import { Edit3, Sparkles, X } from 'lucide-react';
import type { Task, TaskStatus, TaskUpdateV1 } from '../types';

interface TaskEditorProps {
  task: Task;
  disabled: boolean;
  mutationPending: boolean;
  onSave: (update: TaskUpdateV1) => Promise<boolean>;
}

export function TaskEditor({ task, disabled, mutationPending, onSave }: TaskEditorProps) {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState(task.title);
  const [goal, setGoal] = useState(task.goal);
  const [status, setStatus] = useState<TaskStatus>(task.status);
  const [validationError, setValidationError] = useState('');

  useEffect(() => {
    setOpen(false);
    setTitle(task.title);
    setGoal(task.goal);
    setStatus(task.status);
    setValidationError('');
  }, [task.goal, task.id, task.status, task.title]);

  const beginEditing = () => {
    setTitle(task.title);
    setGoal(task.goal);
    setStatus(task.status);
    setValidationError('');
    setOpen(true);
  };

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    const nextTitle = title.trim();
    const nextGoal = goal.trim();
    if (!nextTitle) {
      setValidationError('Task title is required.');
      return;
    }
    const update: TaskUpdateV1 = {};
    if (nextTitle !== task.title) update.title = nextTitle;
    if (nextGoal !== task.goal) update.goal = nextGoal;
    if (status !== task.status) update.status = status;
    if (Object.keys(update).length === 0) {
      setOpen(false);
      return;
    }
    setValidationError('');
    if (await onSave(update)) setOpen(false);
  };

  return (
    <section className="task-editor-shell" aria-labelledby="task-details-title">
      <header>
        <div>
          <span className="task-integrity-kicker">Task integrity</span>
          <h2 id="task-details-title">Task details</h2>
          <p>Edit only the task title, verified goal, and lifecycle.</p>
        </div>
        <button className="secondary-button" type="button" disabled={disabled || mutationPending || open} onClick={beginEditing}>
          <Edit3 size={14} /> Edit task
        </button>
      </header>

      {open ? (
        <form className="task-editor" aria-label={`Edit task ${task.title}`} onSubmit={(event) => void submit(event)}>
          <div className="task-editor-heading">
            <strong>Edit task</strong>
            <button className="icon-button" type="button" aria-label="Close task editor" onClick={() => setOpen(false)}><X size={16} /></button>
          </div>
          {validationError ? <p className="task-editor-error" role="alert">{validationError}</p> : null}
          <fieldset disabled={disabled || mutationPending}>
            <label htmlFor={`task-title-${task.id}`}>Title
              <input id={`task-title-${task.id}`} value={title} onChange={(event) => setTitle(event.target.value)} />
            </label>
            {task.titleSuggestion ? (
              <div className="task-title-suggestion" role="note">
                <Sparkles size={14} aria-hidden="true" />
                <span><strong>Deterministic suggestion</strong><small>Source: {suggestionSource(task.titleSuggestion.source)}</small><code>{task.titleSuggestion.value}</code></span>
                <button className="secondary-button" type="button" onClick={() => setTitle(task.titleSuggestion!.value)}>Use suggestion</button>
              </div>
            ) : null}
            <label htmlFor={`task-goal-${task.id}`}>Goal
              <textarea id={`task-goal-${task.id}`} rows={4} value={goal} onChange={(event) => setGoal(event.target.value)} />
            </label>
            <label htmlFor={`task-lifecycle-${task.id}`}>Status
              <select id={`task-lifecycle-${task.id}`} value={status} onChange={(event) => setStatus(event.target.value as TaskStatus)}>
                <option value="active">Active</option>
                <option value="completed">Completed</option>
                <option value="abandoned">Abandoned</option>
              </select>
            </label>
          </fieldset>
          <footer>
            <button className="secondary-button" type="button" onClick={() => setOpen(false)}>Cancel</button>
            <button className="primary-button" type="submit" disabled={disabled || mutationPending}>Save task</button>
          </footer>
        </form>
      ) : null}
    </section>
  );
}

function suggestionSource(source: Task['titleSuggestion'] extends infer T ? T extends { source: infer S } ? S : never : never) {
  switch (source) {
    case 'branch': return 'verified branch';
    case 'touched_area': return 'verified touched area';
    default: return 'verified goal first line';
  }
}
