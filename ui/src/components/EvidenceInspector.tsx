import { useState } from 'react';
import { Ban, Check, ExternalLink, Info, MoreHorizontal, Pin, RotateCcw, X } from 'lucide-react';
import type { Evidence, Fact, FactStatus } from '../types';
import { FactBadge, FreshnessBadge } from './StatusBadge';

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
  onRevalidate: () => void;
}

const capturedFormatter = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
});

export function EvidenceInspector({ evidence, availableEvidence, fact, replacementFacts, mutationPending, mobileOpen, onClose, onEvidenceSelect, onStatusChange, onRevalidate }: EvidenceInspectorProps) {
  return (
    <aside className={`evidence-inspector ${mobileOpen ? 'mobile-open' : ''}`} aria-label="Evidence inspector">
      <div className="sheet-handle mobile-only" aria-hidden="true" />
      <header className="inspector-header desktop-only">
        <div><strong>Evidence inspector</strong><small>Evidence ID: {evidence.id}</small></div>
        <span><Pin size={15} /><button className="icon-button" type="button" onClick={onClose} aria-label="Close inspector"><X size={17} /></button></span>
      </header>

      <div className="inspector-status desktop-only">
        <FactBadge status={fact.status} />
        <span>{fact.confirmedAt ? `Confirmed on ${capturedFormatter.format(new Date(fact.confirmedAt))}` : 'Awaiting review'}</span>
      </div>

      <section className="mobile-fact-summary mobile-only">
        <span>Evidence &nbsp; <b>{evidence.id.replace('ev_01HZX4C9Y7T2R6D8F3G1K8', 'E-2-')}</b></span>
        <h2>{fact.text}</h2>
      </section>

      <FactActions status={fact.status} replacementFacts={replacementFacts} disabled={mutationPending} onStatusChange={onStatusChange} />

      <section className="inspector-section fact-section desktop-only">
        <h3>Fact</h3>
        <p>{fact.text}</p>
      </section>

      <section className="inspector-section source-section">
        <h3 className="desktop-only">Source</h3>
        <dl>
          <div className="desktop-only"><dt>Evidence</dt><dd><select aria-label="Evidence item" value={evidence.id} onChange={(event) => onEvidenceSelect(event.target.value)}>{availableEvidence.map((item, index) => <option key={item.id} value={item.id}>{index + 1}. {item.source}</option>)}</select></dd></div>
          <div className="desktop-only"><dt>Session</dt><dd><span className="source-value">{evidence.sessionLabel}</span></dd></div>
          <div className="desktop-only"><dt>Turn</dt><dd><span className="source-value">{evidence.turnLabel}</span></dd></div>
          <div className="mobile-only"><dt>Source</dt><dd><span className="source-value">{evidence.source}</span><ExternalLink size={15} /></dd></div>
          <div className="mobile-only"><dt>Captured</dt><dd>{capturedFormatter.format(new Date(evidence.capturedAt))}<FreshnessBadge status={evidence.freshness} /><Info size={15} /></dd></div>
        </dl>
      </section>

      <section className="inspector-section evidence-section">
        <h3>Evidence <span className="desktop-only">(redacted)</span></h3>
        <CodeExcerpt code={evidence.code} />
      </section>

      <section className="inspector-section freshness-section desktop-only">
        <header><h3>Freshness</h3><button className="secondary-button" type="button" disabled={mutationPending} onClick={onRevalidate}>Revalidate</button></header>
        <p><span className={`health-dot health-${evidence.freshness === 'fresh' ? 'good' : 'degraded'}`} /> <strong>{evidence.freshness}</strong> <small>Validated {capturedFormatter.format(new Date(fact.updatedAt))}</small></p>
      </section>

      <section className="inspector-section desktop-only">
        <h3>Selection reason</h3>
        <p>{evidence.selectionReason}</p>
      </section>

      <section className="inspector-section related-files desktop-only">
        <h3>Related files</h3>
        <ul>
          {evidence.relatedFiles.map((file) => (
            <li key={file.path}><span>{file.path}</span><span><em>+{file.additions}</em> <b>−{file.deletions}</b></span></li>
          ))}
        </ul>
      </section>
    </aside>
  );
}

function FactActions({ status, replacementFacts, disabled, onStatusChange }: {
  status: FactStatus;
  replacementFacts: Fact[];
  disabled: boolean;
  onStatusChange: (status: FactStatus, supersedesFactId?: string) => void;
}) {
  const [supersedeOpen, setSupersedeOpen] = useState(false);
  const [replacementId, setReplacementId] = useState(replacementFacts[0]?.id ?? '');

  return (
    <div className="fact-actions-wrap">
      <div className="fact-actions" role="group" aria-label="Fact review actions">
        <button disabled={disabled} className={status === 'confirmed' ? 'active' : ''} type="button" onClick={() => onStatusChange('confirmed')}><Check size={16} /> Confirmed</button>
        <button disabled={disabled} className={status === 'pinned' ? 'active' : ''} type="button" onClick={() => onStatusChange('pinned')}><Pin size={16} /> Pin</button>
        <button disabled={disabled} className={status === 'invalid' ? 'danger active' : 'danger'} type="button" onClick={() => onStatusChange('invalid')}><Ban size={16} /> Invalidate</button>
        <button disabled={disabled || replacementFacts.length === 0} className="desktop-only" type="button" onClick={() => setSupersedeOpen((open) => !open)}><RotateCcw size={16} /> Supersede</button>
        <button disabled={disabled || replacementFacts.length === 0} className="mobile-only more-action" type="button" aria-label="Supersede fact" onClick={() => setSupersedeOpen((open) => !open)}><MoreHorizontal size={17} /></button>
      </div>
      {supersedeOpen ? (
        <div className="supersede-picker">
          <label htmlFor="supersede-fact">Replacement fact</label>
          <select id="supersede-fact" value={replacementId} onChange={(event) => setReplacementId(event.target.value)}>
            {replacementFacts.map((candidate) => <option key={candidate.id} value={candidate.id}>{candidate.text}</option>)}
          </select>
          <button className="secondary-button" type="button" disabled={disabled || !replacementId} onClick={() => { onStatusChange('superseded', replacementId); setSupersedeOpen(false); }}>Apply</button>
        </div>
      ) : null}
    </div>
  );
}

function CodeExcerpt({ code }: { code: string }) {
  return (
    <pre className="code-excerpt" tabIndex={0} aria-label="Redacted evidence code">
      {code.split('\n').map((line, index) => <span key={`${index}-${line}`}><i>{index + 1}</i><code>{line}</code></span>)}
    </pre>
  );
}
