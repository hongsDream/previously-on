import { AlertTriangle, Box, CheckCircle2, ChevronDown, ChevronUp } from 'lucide-react';
import type { BootstrapData, Checkpoint } from '../types';

interface ContextPackPreviewProps {
  checkpoint: Checkpoint;
  contextPack: BootstrapData['contextPacks'][string];
  expanded: boolean;
  onToggle: () => void;
}

export function ContextPackPreview({ checkpoint, contextPack, expanded, onToggle }: ContextPackPreviewProps) {
  const percent = Math.min(100, Math.round((contextPack.token_count / contextPack.token_budget) * 100));
  return (
    <section className={`context-pack ${expanded ? 'expanded' : ''}`} aria-labelledby="context-pack-title">
      <button className="context-pack-bar" type="button" onClick={onToggle} aria-expanded={expanded}>
        <span className="context-pack-title">
          {expanded ? <ChevronUp size={16} /> : <Box size={19} />}
          <strong id="context-pack-title">Context pack<span className="mobile-only"> (Checkpoint {checkpoint.sequence})</span><span className="desktop-only"> preview</span></strong>
        </span>
        <span className="token-meter desktop-only">
          <small>Token estimate</small>
          <b>{contextPack.token_count.toLocaleString()} tokens</b>
          <small>Budget</small>
          <b>{contextPack.token_budget.toLocaleString()}</b>
          <i><span style={{ width: `${percent}%` }} /></i>
          <small>{percent}%</small>
        </span>
        <span className="mobile-only mobile-token-count">{contextPack.token_count.toLocaleString()} tokens</span>
        {expanded ? <ChevronUp size={17} /> : <ChevronDown size={17} />}
      </button>

      {expanded ? (
        <>
          {contextPack.coverage.status !== 'complete' ? (
            <div className="pack-coverage-warning" role="status">
              <AlertTriangle size={14} />
              <span><strong>{contextPack.coverage.status} capture</strong>{[...contextPack.coverage.missing, ...contextPack.coverage.warnings].join(' · ')}</span>
            </div>
          ) : null}
          <div className="context-pack-content">
            <PackSection title="Goal" count={contextPack.goal ? '1 item' : '0 items'}>
              <p>{contextPack.goal ?? 'No verified goal was selected.'}</p>
            </PackSection>
            <PackSection title="Facts" count={`${contextPack.facts.length} items`}>
              {contextPack.facts.map((fact) => <PackText key={fact.id} label={fact.kind} text={fact.content} />)}
            </PackSection>
            <PackSection title="Open items" count={`${contextPack.unresolved_items.length} items`}>
              {contextPack.unresolved_items.map((fact) => <PackText key={fact.id} label="open" text={fact.content} />)}
            </PackSection>
            <PackSection title="Files" count={`${contextPack.files.length} files`}>
              {contextPack.files.map((file) => <PackText key={`${file.path}-${file.status}`} label={file.attribution} text={file.path} />)}
            </PackSection>
            <PackSection title="Tests" count={`${contextPack.tests.length} tests`}>
              {contextPack.tests.map((test) => <PackText key={`${test.name}-${test.status}`} icon={<CheckCircle2 size={13} className={test.status === 'passed' ? 'success-text' : 'warning'} />} label={test.status} text={test.name} />)}
            </PackSection>
          </div>
        </>
      ) : null}
    </section>
  );
}

function PackSection({ title, count, children }: { title: string; count: string; children: React.ReactNode }) {
  return (
    <div className="pack-section">
      <header><strong>{title}</strong><span>{count}</span></header>
      <div>{children}</div>
    </div>
  );
}

function PackText({ icon, label, text }: { icon?: React.ReactNode; label: string; text: string }) {
  return <span className="pack-text">{icon}<small>{label.replaceAll('_', ' ')}</small><span>{text}</span></span>;
}
