import { useState } from 'react';
import { Ban, Check, ExternalLink, Info, MoreHorizontal, Pencil, Pin, RotateCcw, X } from 'lucide-react';
import type { Evidence, Fact, FactStatus } from '../types';
import { FactBadge, FreshnessBadge } from './StatusBadge';
import { useI18n } from '../i18n-context';

interface EvidenceInspectorProps {
  evidence: Evidence;
  availableEvidence: Evidence[];
  fact: Fact;
  mobileOpen: boolean;
  onClose: () => void;
  onEvidenceSelect: (evidenceId: string) => void;
  replacementFacts: Fact[];
  mutationPending: boolean;
  onStatusChange: (status: FactStatus, supersedesFactId?: string) => void;
  onFactUpdate: (content: string, deprecatedAfterCommit: string) => Promise<boolean>;
  onSessionExcludedChange: (excluded: boolean) => void;
  onRevalidate: () => void;
}

export function EvidenceInspector({ evidence, availableEvidence, fact, replacementFacts, mutationPending, mobileOpen, onClose, onEvidenceSelect, onStatusChange, onFactUpdate, onSessionExcludedChange, onRevalidate }: EvidenceInspectorProps) {
  const { t, locale } = useI18n();
  const capturedFormatter = new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric', year: 'numeric', hour: '2-digit', minute: '2-digit' });
  const evidenceIndex = availableEvidence.findIndex((item) => item.id === evidence.id);
  const evidenceSequence = evidenceIndex >= 0 ? evidenceIndex + 1 : 1;
  return (
    <aside className={`evidence-inspector ${mobileOpen ? 'mobile-open' : ''}`} aria-label={t('Evidence inspector')}>
      <div className="sheet-handle mobile-only" aria-hidden="true" />
      <header className="inspector-header desktop-only">
        <div><strong>{t('Evidence inspector')}</strong><small>{t('Evidence ID: {id}', { id: evidence.id })}</small></div>
        <span><Pin size={15} /><button className="icon-button" type="button" onClick={onClose} aria-label={t('Close inspector')}><X size={17} /></button></span>
      </header>

      <div className="inspector-status desktop-only">
        <FactBadge status={fact.status} />
        <span>{fact.confirmedAt ? t('Confirmed on {date}', { date: capturedFormatter.format(new Date(fact.confirmedAt)) }) : t('Awaiting review')}</span>
      </div>

      <section className="mobile-fact-summary mobile-only">
        <span>{t('Evidence')} &nbsp; <b>E-{evidenceSequence}</b></span>
        <h2>{fact.text}</h2>
      </section>

      <FactActions status={fact.status} replacementFacts={replacementFacts} disabled={mutationPending} onStatusChange={onStatusChange} />

      <FactEditor fact={fact} disabled={mutationPending} onSave={onFactUpdate} />

      <section className="inspector-section source-section">
        <h3 className="desktop-only">{t('Source')}</h3>
        <dl>
          <div className="desktop-only"><dt>{t('Evidence')}</dt><dd><select aria-label={t('Evidence item')} value={evidence.id} onChange={(event) => onEvidenceSelect(event.target.value)}>{availableEvidence.map((item, index) => <option key={item.id} value={item.id}>{index + 1}. {item.source}</option>)}</select></dd></div>
          <div className="desktop-only"><dt>{t('Session')}</dt><dd><span className="source-value">{evidence.sessionLabel}</span></dd></div>
          <div className="desktop-only"><dt>{t('Turn')}</dt><dd><span className="source-value">{evidence.turnLabel}</span></dd></div>
          <div className="mobile-only"><dt>{t('Source')}</dt><dd><span className="source-value">{evidence.source}</span><ExternalLink size={15} /></dd></div>
          <div className="mobile-only"><dt>{t('Captured')}</dt><dd>{capturedFormatter.format(new Date(evidence.capturedAt))}<FreshnessBadge status={evidence.freshness} /><Info size={15} /></dd></div>
        </dl>
        <div className={`session-memory-control desktop-only ${evidence.excludedSession ? 'session-memory-excluded' : ''}`}>
          <span>{evidence.excludedSession ? t('This session is excluded from future Context Packs.') : t('This session can contribute verified facts to Context Packs.')}</span>
          <button className="secondary-button" type="button" disabled={mutationPending || !evidence.sessionId} onClick={() => onSessionExcludedChange(!evidence.excludedSession)}>
            {evidence.excludedSession ? t('Include session') : t('Exclude session')}
          </button>
        </div>
      </section>

      <section className="inspector-section evidence-section">
        <h3>{t('Evidence')} <span className="desktop-only">{t('(redacted)')}</span></h3>
        <CodeExcerpt code={evidence.code} />
      </section>

      <section className="inspector-section freshness-section desktop-only">
        <header><h3>{t('Freshness')}</h3><button className="secondary-button" type="button" disabled={mutationPending} onClick={onRevalidate}>{t('Revalidate')}</button></header>
        <p><span className={`health-dot health-${evidence.freshness === 'fresh' ? 'good' : 'degraded'}`} /> <strong>{t(evidence.freshness)}</strong> <small>{t('Validated {date}', { date: capturedFormatter.format(new Date(fact.updatedAt)) })}</small></p>
      </section>

      <section className="inspector-section desktop-only">
        <h3>{t('Selection reason')}</h3>
        <p>{evidence.selectionReason}</p>
      </section>

      <section className="inspector-section related-files desktop-only">
        <h3>{t('Related files')}</h3>
        <ul>
          {evidence.relatedFiles.map((file) => (
            <li key={file.path}><span>{file.path}</span><span><em>+{file.additions}</em> <b>−{file.deletions}</b></span></li>
          ))}
        </ul>
      </section>
    </aside>
  );
}

function FactEditor({ fact, disabled, onSave }: { fact: Fact; disabled: boolean; onSave: (content: string, deprecatedAfterCommit: string) => Promise<boolean> }) {
  const { t } = useI18n();
  const [editing, setEditing] = useState(false);
  const [content, setContent] = useState(fact.text);
  const [deprecatedAfterCommit, setDeprecatedAfterCommit] = useState(fact.deprecatedAfterCommit ?? '');
  const [error, setError] = useState('');

  const save = async () => {
    const nextContent = content.trim();
    if (!nextContent) {
      setError(t('Fact text is required.'));
      return;
    }
    const saved = await onSave(nextContent, deprecatedAfterCommit.trim());
    if (saved) {
      setError('');
      setEditing(false);
    }
  };

  if (!editing) {
    return (
      <section className="inspector-section fact-section desktop-only">
        <header><h3>{t('Fact')}</h3><button className="secondary-button" type="button" disabled={disabled} onClick={() => { setContent(fact.text); setDeprecatedAfterCommit(fact.deprecatedAfterCommit ?? ''); setEditing(true); }}><Pencil size={12} /> {t('Edit')}</button></header>
        <p>{fact.text}</p>
        <small className="fact-selection-copy">{fact.selectionReason ?? t('Not selected in the current verified Context Pack.')}</small>
        {fact.deprecatedAfterCommit ? <small className="fact-deprecation-copy">{t('Treat as stale after commit {sha}', { sha: fact.deprecatedAfterCommit })}</small> : null}
      </section>
    );
  }

  return (
    <section className="inspector-section fact-section fact-editor desktop-only">
      <header><h3>{t('Edit fact memory')}</h3></header>
      <label htmlFor="fact-memory-text">{t('Fact text')}</label>
      <textarea id="fact-memory-text" rows={4} maxLength={500} value={content} onChange={(event) => setContent(event.target.value)} />
      <label htmlFor="fact-deprecation-commit">{t('Deprecate after Git commit')} <span>{t('(optional)')}</span></label>
      <input id="fact-deprecation-commit" value={deprecatedAfterCommit} onChange={(event) => setDeprecatedAfterCommit(event.target.value)} placeholder={t('7–64 character SHA')} />
      {error ? <p className="fact-editor-error" role="alert">{error}</p> : null}
      <footer>
        <button className="secondary-button" type="button" disabled={disabled} onClick={() => setEditing(false)}>{t('Cancel')}</button>
        <button className="primary-button" type="button" disabled={disabled} onClick={() => void save()}>{t('Save memory')}</button>
      </footer>
    </section>
  );
}

function FactActions({ status, replacementFacts, disabled, onStatusChange }: {
  status: FactStatus;
  replacementFacts: Fact[];
  disabled: boolean;
  onStatusChange: (status: FactStatus, supersedesFactId?: string) => void;
}) {
  const { t } = useI18n();
  const [supersedeOpen, setSupersedeOpen] = useState(false);
  const [replacementId, setReplacementId] = useState(replacementFacts[0]?.id ?? '');

  return (
    <div className="fact-actions-wrap">
      <div className="fact-actions" role="group" aria-label={t('Fact review actions')}>
        <button disabled={disabled} className={status === 'confirmed' ? 'active' : ''} type="button" onClick={() => onStatusChange('confirmed')}><Check size={16} /> {t('Confirmed')}</button>
        <button disabled={disabled} className={status === 'pinned' ? 'active' : ''} type="button" onClick={() => onStatusChange('pinned')}><Pin size={16} /> {t('Pin')}</button>
        <button disabled={disabled} className={status === 'invalid' ? 'danger active' : 'danger'} type="button" onClick={() => onStatusChange('invalid')}><Ban size={16} /> {t('Invalidate')}</button>
        <button disabled={disabled || replacementFacts.length === 0} className="desktop-only" type="button" onClick={() => setSupersedeOpen((open) => !open)}><RotateCcw size={16} /> {t('Supersede')}</button>
        <button disabled={disabled || replacementFacts.length === 0} className="mobile-only more-action" type="button" aria-label={t('Supersede fact')} onClick={() => setSupersedeOpen((open) => !open)}><MoreHorizontal size={17} /></button>
      </div>
      {supersedeOpen ? (
        <div className="supersede-picker">
          <label htmlFor="supersede-fact">{t('Replacement fact')}</label>
          <select id="supersede-fact" value={replacementId} onChange={(event) => setReplacementId(event.target.value)}>
            {replacementFacts.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.text}</option>)}
          </select>
          <button className="secondary-button" type="button" disabled={disabled || !replacementId} onClick={() => { onStatusChange('superseded', replacementId); setSupersedeOpen(false); }}>{t('Apply')}</button>
        </div>
      ) : null}
    </div>
  );
}

function CodeExcerpt({ code }: { code: string }) {
  const { t } = useI18n();
  return (
    <pre className="code-excerpt" tabIndex={0} aria-label={t('Redacted evidence code')}>
      {code.split('\n').map((line, index) => <span key={`${index}-${line}`}><i>{index + 1}</i><code>{line}</code></span>)}
    </pre>
  );
}
