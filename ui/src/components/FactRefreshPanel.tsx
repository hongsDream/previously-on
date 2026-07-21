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
import { useI18n } from '../i18n-context';

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
  const { t } = useI18n();
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
        if (!cancelled) setError(caught instanceof Error ? caught.message : t('Refresh status could not be checked.'));
      });
    }, 750);
    return () => {
      cancelled = true;
      controller.abort();
      window.clearTimeout(timer);
    };
  }, [onPoll, operation, running, t]);

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
          <span className="task-integrity-kicker">{t('Optional beta')}</span>
          <h2 id="fact-refresh-title"><Bot size={16} /> {t('AI-assisted fact refresh')}</h2>
          <p>{t('Runs only from this button and returns review candidates—not Evidence or confirmed facts.')}</p>
        </div>
        <button className="primary-button" type="button" disabled={!ready || disabled || mutationPending || running} onClick={() => void start()}>
          <RefreshCw size={14} className={running ? 'spin-icon' : ''} /> {running ? t('Refreshing…') : t('Refresh facts')}
        </button>
      </header>

      {!ready ? (
        <div className={`refresh-capability-message capability-${capability.status}`} role="status">
          <CircleAlert size={16} />
          <span><strong>{t(capabilityLabel(capability.status))}</strong>{capability.reason ? t(capability.reason) : t('The required input-only permission profile has not been verified.')}</span>
        </div>
      ) : (
        <div className="refresh-capability-message capability-ready" role="status">
          <ShieldCheck size={16} />
          <span><strong>{t('Input-only profile verified')}</strong><code>{capability.profileName}</code>{t(' · network disabled · approval never')}</span>
        </div>
      )}

      {error ? <p className="fact-refresh-error" role="alert">{error}</p> : null}
      {operation ? <RefreshOperation operation={operation} disabled={disabled || mutationPending} onReview={review} /> : (
        <p className="fact-refresh-empty">{t('No AI refresh has been requested for this task. Existing local facts remain unchanged.')}</p>
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
  const { t } = useI18n();
  return (
    <div className="refresh-operation" aria-live="polite">
      <header>
        <span><strong>{t('Refresh status')}</strong><code>{operation.operationId}</code></span>
        <span className={`refresh-operation-state refresh-${operation.status}`}>{t(operation.status.replace('_', ' '))}</span>
      </header>
      {operation.status === 'failed' ? <p className="fact-refresh-error" role="alert">{operation.error || t('The local refresh failed without exposing a model response.')}</p> : null}
      {operation.status === 'pending' || operation.status === 'thread_created' ? (
        <p className="fact-refresh-progress"><RefreshCw size={14} className="spin-icon" /> {t('Waiting for the isolated local operation. You may leave this task and return later.')}</p>
      ) : null}
      {operation.status === 'completed' ? (
        <section className="fact-candidate-review" aria-labelledby={`fact-candidates-${operation.operationId}`}>
          <header>
            <div><h3 id={`fact-candidates-${operation.operationId}`}>{t('Fact candidates')}</h3><p>{t('Review every suggestion. Accepting creates a candidate only; it does not create Evidence.')}</p></div>
            <span>{t('{count} pending', { count: operation.candidates.filter((candidate) => candidate.status === 'pending').length })}</span>
          </header>
          {operation.candidates.length ? (
            <ul>{operation.candidates.map((candidate) => <CandidateCard key={candidate.id} candidate={candidate} disabled={disabled} onReview={onReview} />)}</ul>
          ) : <p className="fact-refresh-empty">{t('The model returned no valid add, update, or deprecate candidates.')}</p>}
        </section>
      ) : null}
      <dl className="refresh-metrics" aria-label={t('Exposed refresh metrics')}>
        <div><dt>{t('Model')}</dt><dd>{operation.modelId || t('Unavailable')}</dd></div>
        <div><dt>{t('Input tokens')}</dt><dd>{formatMetric(operation.inputTokens, t('Unavailable'))}</dd></div>
        <div><dt>{t('Output tokens')}</dt><dd>{formatMetric(operation.outputTokens, t('Unavailable'))}</dd></div>
        <div><dt>{t('Latency')}</dt><dd>{operation.latencyMs === undefined || operation.latencyMs === null ? t('Unavailable') : `${operation.latencyMs} ms`}</dd></div>
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
  const { t } = useI18n();
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
        <span className="candidate-review-state">{candidate.status === 'accepted' ? t('Fact Candidate') : t(candidate.status)}</span>
      </header>
      {editing ? (
        <fieldset disabled={disabled}>
          <legend className="sr-only">{t('Edit candidate')}</legend>
          <label>{t('Fact kind')}
            <select value={kind} onChange={(event) => setKind(event.target.value as FactKind)}>{factKinds.map((item) => <option key={item} value={item}>{t(item.replace('_', ' '))}</option>)}</select>
          </label>
          <label>{t('Candidate text')}
            <textarea rows={4} value={content} onChange={(event) => setContent(event.target.value)} />
          </label>
        </fieldset>
      ) : <p>{candidate.content}</p>}
      <small>{candidate.reason}</small>
      {candidate.factId ? <code>{t('Existing fact {id}', { id: candidate.factId })}</code> : null}
      {pending ? (
        <footer>
          {editing ? <button className="secondary-button" type="button" onClick={() => { setEditing(false); setContent(candidate.content); setKind(candidate.kind); }}><X size={13} /> {t('Cancel edit')}</button>
            : <button className="secondary-button" type="button" disabled={disabled} onClick={() => setEditing(true)}><Pencil size={13} /> {t('Edit')}</button>}
          <button className="secondary-button" type="button" disabled={disabled} onClick={() => void onReview(candidate, 'reject')}><X size={13} /> {t('Reject')}</button>
          <button className="primary-button" type="button" disabled={disabled || !content.trim()} onClick={() => void accept()}><Check size={13} /> {candidate.action === 'deprecate' ? t('Accept deprecation candidate') : t('Accept as Fact Candidate')}</button>
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

function formatMetric(value: number | null | undefined, unavailable: string) {
  return value === undefined || value === null ? unavailable : value.toLocaleString();
}

function createRequestId(taskId: string) {
  const randomId = globalThis.crypto?.randomUUID?.() ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `fact-refresh-${taskId}-${randomId}`;
}
