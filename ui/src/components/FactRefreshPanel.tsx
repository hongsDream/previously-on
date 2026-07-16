import { useEffect, useState } from 'react';
import { Bot, Check, CircleAlert, Pencil, RefreshCw, ShieldCheck, X } from 'lucide-react';
import type { FactCandidateReviewResponse } from '../lib/api';
import type {
  AiFactCandidateV1,
  AiFactRefreshOperationV1,
  AiRefreshCapabilityV1,
  FactKind,
  Task,
} from '../types';

interface FactRefreshPanelProps {
  task: Task;
  capability: AiRefreshCapabilityV1;
  initialOperation?: AiFactRefreshOperationV1;
  disabled: boolean;
  mutationPending: boolean;
  onStart: (requestId: string) => Promise<AiFactRefreshOperationV1 | null>;
  onPoll: (operationId: string, signal: AbortSignal) => Promise<AiFactRefreshOperationV1 | null>;
  onReview: (
    operationId: string,
    candidateId: string,
    decision: 'accept' | 'reject',
    content?: string,
    kind?: FactKind,
  ) => Promise<FactCandidateReviewResponse | null>;
}

const factKinds: FactKind[] = ['decision', 'constraint', 'open_item', 'progress', 'goal', 'note'];

export function FactRefreshPanel({
  task,
  capability,
  initialOperation,
  disabled,
  mutationPending,
  onStart,
  onPoll,
  onReview,
}: FactRefreshPanelProps) {
  const [operation, setOperation] = useState(initialOperation);
  const [error, setError] = useState('');
  const ready = capability.status === 'ready';
  const running = operation?.status === 'pending' || operation?.status === 'thread_created';

  useEffect(() => {
    setOperation(initialOperation);
  }, [initialOperation]);

  useEffect(() => {
    if (!operation || !running) return;
    let cancelled = false;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void onPoll(operation.operationId, controller.signal).then((next) => {
        if (!cancelled && next) {
          setOperation(next);
          setError('');
        }
      }).catch((caught: unknown) => {
        if (!cancelled) setError(caught instanceof Error ? caught.message : 'Refresh status could not be checked.');
      });
    }, 750);
    return () => {
      cancelled = true;
      controller.abort();
      window.clearTimeout(timer);
    };
  }, [onPoll, operation, running]);

  const start = async () => {
    if (!ready || disabled || mutationPending || running) return;
    setError('');
    const next = await onStart(createRequestId(task.id));
    if (next) setOperation(next);
  };

  const review = async (
    candidate: AiFactCandidateV1,
    decision: 'accept' | 'reject',
    content?: string,
    kind?: FactKind,
  ) => {
    if (!operation) return false;
    setError('');
    const result = await onReview(operation.operationId, candidate.id, decision, content, kind);
    if (!result) return false;
    setOperation((current) => current ? {
      ...current,
      candidates: current.candidates.map((item) => item.id === candidate.id ? result.candidate : item),
    } : current);
    return true;
  };

  return (
    <section className="fact-refresh-panel" aria-labelledby="fact-refresh-title">
      <header>
        <div>
          <span className="task-integrity-kicker">Optional beta</span>
          <h2 id="fact-refresh-title"><Bot size={16} /> AI-assisted fact refresh</h2>
          <p>Runs only from this button and returns review candidates—not Evidence or confirmed facts.</p>
        </div>
        <button className="primary-button" type="button" disabled={!ready || disabled || mutationPending || running} onClick={() => void start()}>
          <RefreshCw size={14} className={running ? 'spin-icon' : ''} /> {running ? 'Refreshing…' : 'Refresh facts'}
        </button>
      </header>

      {!ready ? (
        <div className={`refresh-capability-message capability-${capability.status}`} role="status">
          <CircleAlert size={16} />
          <span><strong>{capabilityLabel(capability.status)}</strong>{capability.reason || 'The required input-only permission profile has not been verified.'}</span>
        </div>
      ) : (
        <div className="refresh-capability-message capability-ready" role="status">
          <ShieldCheck size={16} />
          <span><strong>Input-only profile verified</strong><code>{capability.profileName}</code> · network disabled · approval never</span>
        </div>
      )}

      {error ? <p className="fact-refresh-error" role="alert">{error}</p> : null}
      {operation ? <RefreshOperation operation={operation} disabled={disabled || mutationPending} onReview={review} /> : (
        <p className="fact-refresh-empty">No AI refresh has been requested for this task. Existing local facts remain unchanged.</p>
      )}
    </section>
  );
}

function RefreshOperation({
  operation,
  disabled,
  onReview,
}: {
  operation: AiFactRefreshOperationV1;
  disabled: boolean;
  onReview: (candidate: AiFactCandidateV1, decision: 'accept' | 'reject', content?: string, kind?: FactKind) => Promise<boolean>;
}) {
  return (
    <div className="refresh-operation" aria-live="polite">
      <header>
        <span><strong>Refresh status</strong><code>{operation.operationId}</code></span>
        <span className={`refresh-operation-state refresh-${operation.status}`}>{operation.status.replace('_', ' ')}</span>
      </header>
      {operation.status === 'failed' ? <p className="fact-refresh-error" role="alert">{operation.error || 'The local refresh failed without exposing a model response.'}</p> : null}
      {operation.status === 'pending' || operation.status === 'thread_created' ? (
        <p className="fact-refresh-progress"><RefreshCw size={14} className="spin-icon" /> Waiting for the isolated local operation. You may leave this task and return later.</p>
      ) : null}
      {operation.status === 'completed' ? (
        <section className="fact-candidate-review" aria-labelledby={`fact-candidates-${operation.operationId}`}>
          <header>
            <div><h3 id={`fact-candidates-${operation.operationId}`}>Fact candidates</h3><p>Review every suggestion. Accepting creates a candidate only; it does not create Evidence.</p></div>
            <span>{operation.candidates.filter((candidate) => candidate.status === 'pending').length} pending</span>
          </header>
          {operation.candidates.length ? (
            <ul>{operation.candidates.map((candidate) => <CandidateCard key={candidate.id} candidate={candidate} disabled={disabled} onReview={onReview} />)}</ul>
          ) : <p className="fact-refresh-empty">The model returned no valid add, update, or deprecate candidates.</p>}
        </section>
      ) : null}
      <dl className="refresh-metrics" aria-label="Exposed refresh metrics">
        <div><dt>Model</dt><dd>{operation.modelId || 'Unavailable'}</dd></div>
        <div><dt>Input tokens</dt><dd>{formatMetric(operation.inputTokens)}</dd></div>
        <div><dt>Output tokens</dt><dd>{formatMetric(operation.outputTokens)}</dd></div>
        <div><dt>Latency</dt><dd>{operation.latencyMs === undefined || operation.latencyMs === null ? 'Unavailable' : `${operation.latencyMs} ms`}</dd></div>
      </dl>
    </div>
  );
}

function CandidateCard({
  candidate,
  disabled,
  onReview,
}: {
  candidate: AiFactCandidateV1;
  disabled: boolean;
  onReview: (candidate: AiFactCandidateV1, decision: 'accept' | 'reject', content?: string, kind?: FactKind) => Promise<boolean>;
}) {
  const [editing, setEditing] = useState(false);
  const [content, setContent] = useState(candidate.content);
  const [kind, setKind] = useState<FactKind>(candidate.kind);
  const pending = candidate.status === 'pending';

  const accept = async () => {
    const saved = await onReview(candidate, 'accept', content.trim(), kind);
    if (saved) setEditing(false);
  };

  return (
    <li className={`fact-refresh-candidate candidate-${candidate.status}`}>
      <header>
        <span className={`candidate-action action-${candidate.action}`}>{candidate.action}</span>
        <span className="candidate-kind">{candidate.kind.replace('_', ' ')}</span>
        <span className="candidate-review-state">{candidate.status === 'accepted' ? 'Fact Candidate' : candidate.status}</span>
      </header>
      {editing ? (
        <fieldset disabled={disabled}>
          <legend className="sr-only">Edit candidate</legend>
          <label>Fact kind
            <select value={kind} onChange={(event) => setKind(event.target.value as FactKind)}>{factKinds.map((item) => <option key={item} value={item}>{item.replace('_', ' ')}</option>)}</select>
          </label>
          <label>Candidate text
            <textarea rows={4} value={content} onChange={(event) => setContent(event.target.value)} />
          </label>
        </fieldset>
      ) : <p>{candidate.content}</p>}
      <small>{candidate.reason}</small>
      {candidate.factId ? <code>Existing fact {candidate.factId}</code> : null}
      {pending ? (
        <footer>
          {editing ? <button className="secondary-button" type="button" onClick={() => { setEditing(false); setContent(candidate.content); setKind(candidate.kind); }}><X size={13} /> Cancel edit</button>
            : <button className="secondary-button" type="button" disabled={disabled} onClick={() => setEditing(true)}><Pencil size={13} /> Edit</button>}
          <button className="secondary-button" type="button" disabled={disabled} onClick={() => void onReview(candidate, 'reject')}><X size={13} /> Reject</button>
          <button className="primary-button" type="button" disabled={disabled || !content.trim()} onClick={() => void accept()}><Check size={13} /> {candidate.action === 'deprecate' ? 'Accept deprecation candidate' : 'Accept as Fact Candidate'}</button>
        </footer>
      ) : null}
    </li>
  );
}

function capabilityLabel(status: AiRefreshCapabilityV1['status']) {
  switch (status) {
    case 'needs_setup': return 'Setup required';
    case 'unsupported': return 'App Server unsupported';
    case 'blocked': return 'Refresh blocked';
    default: return 'Ready';
  }
}

function formatMetric(value?: number | null) {
  return value === undefined || value === null ? 'Unavailable' : value.toLocaleString();
}

function createRequestId(taskId: string) {
  const randomId = globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `fact-refresh-${taskId}-${randomId}`;
}
