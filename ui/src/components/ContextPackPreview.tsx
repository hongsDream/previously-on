import { AlertTriangle, Box, CheckCircle2, ChevronDown, ChevronUp, GitCompareArrows } from 'lucide-react';
import type { BootstrapData, Checkpoint, RelatedChange, TemporalStatus } from '../types';

interface ContextPackPreviewProps {
  checkpoint: Checkpoint;
  contextPack: BootstrapData['contextPacks'][string];
  expanded: boolean;
  onToggle: () => void;
}

function shortSha(value: string | undefined) {
  return value ? value.slice(0, 8) : 'unknown';
}

function temporalLabel(status: TemporalStatus) {
  switch (status) {
    case 'unchanged': return 'No relevant code changed';
    case 'changed': return 'Relevant code changed';
    case 'diverged': return 'Git history diverged';
    case 'broken': return 'Related code was removed';
    case 'degraded': return 'Current state could not be fully verified';
  }
}

function changeText(change: RelatedChange) {
  if (change.status === 'renamed' && change.previousPath) return `${change.previousPath} → ${change.path}`;
  return change.path;
}

export function ContextPackPreview({ checkpoint, contextPack, expanded, onToggle }: ContextPackPreviewProps) {
  const percent = Math.min(100, Math.round((contextPack.token_count / contextPack.token_budget) * 100));
  const temporal = contextPack.temporal_revalidation ?? (checkpoint.temporalRevalidation ? {
    status: checkpoint.temporalRevalidation.status,
    baseline_head: checkpoint.temporalRevalidation.baselineSha,
    current_head: checkpoint.temporalRevalidation.currentSha,
    related_changes: checkpoint.temporalRevalidation.changes?.map((change) => ({
      path: change.path,
      previous_path: change.previousPath,
      status: change.status,
      additions: change.additions,
      deletions: change.deletions,
    })),
    warnings: checkpoint.temporalRevalidation.warnings,
  } : undefined);
  const changes: RelatedChange[] = (temporal?.related_changes ?? []).map((change) => ({
    path: change.path,
    previousPath: change.previous_path,
    status: change.status,
    additions: change.additions,
    deletions: change.deletions,
  }));
  const warnings = [
    ...(temporal?.warnings ?? []),
    ...(contextPack.current_validation?.warnings ?? []),
    ...contextPack.coverage.warnings,
  ];
  const reviewCount = contextPack.unresolved_items.length + warnings.length + (temporal && temporal.status !== 'unchanged' ? 1 : 0);

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
          {temporal ? (
            <div className={`pack-temporal-summary temporal-${temporal.status}`}>
              <GitCompareArrows size={15} />
              <strong>{temporalLabel(temporal.status)}</strong>
              <code>{shortSha(temporal.baseline_head ?? checkpoint.sha)}</code>
              <span aria-hidden="true">→</span>
              <code>{shortSha(temporal.current_head ?? temporal.baseline_head ?? checkpoint.sha)}</code>
            </div>
          ) : null}
          <div className="context-pack-content">
            <PackSection title="Then" count={`${contextPack.facts.length + (contextPack.goal ? 1 : 0)} items`}>
              {contextPack.goal ? <PackText label="goal" text={contextPack.goal} /> : <p>No verified goal was selected.</p>}
              {contextPack.facts.map((fact) => <PackText key={fact.id} label={fact.kind} text={fact.content} />)}
            </PackSection>
            <PackSection title="Since" count={`${changes.length || contextPack.files.length} files`}>
              {changes.length > 0
                ? changes.map((change) => <PackText key={`${change.previousPath ?? ''}-${change.path}-${change.status}`} label={change.status} text={changeText(change)} />)
                : contextPack.files.map((file) => <PackText key={`${file.path}-${file.status}`} label={file.status} text={file.path} />)}
              {changes.length === 0 && contextPack.files.length === 0 ? <p>No relevant file changes were selected.</p> : null}
            </PackSection>
            <PackSection title="Now" count={`${contextPack.tests.length} tests`}>
              {contextPack.current_validation ? (
                <PackText
                  label={contextPack.current_validation.status}
                  text={`${(contextPack.current_validation.verified_paths ?? []).length} verified paths at ${shortSha(contextPack.current_validation.current_head)}`}
                />
              ) : null}
              {contextPack.tests.map((test) => <PackText key={`${test.name}-${test.status}`} icon={<CheckCircle2 size={13} className={test.status === 'passed' ? 'success-text' : 'warning'} />} label={test.status} text={test.name} />)}
              {!contextPack.current_validation && contextPack.tests.length === 0 ? <p>No current validation result was selected.</p> : null}
            </PackSection>
            <PackSection title="Needs review" count={`${reviewCount} items`}>
              {temporal && temporal.status !== 'unchanged' ? <PackText label={temporal.status} text={temporalLabel(temporal.status)} /> : null}
              {contextPack.unresolved_items.map((fact) => <PackText key={fact.id} label="open" text={fact.content} />)}
              {warnings.map((warning, index) => <PackText key={`${warning}-${index}`} label="warning" text={warning} />)}
              {reviewCount === 0 ? <p>No unresolved or stale items were selected.</p> : null}
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
