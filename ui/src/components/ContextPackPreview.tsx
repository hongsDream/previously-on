import { AlertTriangle, Box, CheckCircle2, ChevronDown, ChevronUp, GitCompareArrows } from 'lucide-react';
import type { BootstrapData, Checkpoint, RelatedChange, TemporalStatus } from '../types';
import { useI18n } from '../i18n-context';

interface ContextPackPreviewProps {
  checkpoint: Checkpoint;
  contextPack: BootstrapData['contextPacks'][string];
  expanded: boolean;
  onToggle: () => void;
}

function shortSha(value: string | undefined, unknown: string) {
  return value ? value.slice(0, 8) : unknown;
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
  const { t, locale } = useI18n();
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
          <strong id="context-pack-title">{t('Context pack')}<span className="mobile-only"> {t('(Checkpoint {sequence})', { sequence: checkpoint.sequence })}</span><span className="desktop-only"> {t('preview')}</span></strong>
        </span>
        <span className="token-meter desktop-only">
          <small>{t('Token estimate')}</small>
          <b>{t('{count} tokens', { count: contextPack.token_count.toLocaleString(locale) })}</b>
          <small>{t('Budget')}</small>
          <b>{contextPack.token_budget.toLocaleString(locale)}</b>
          <i><span style={{ width: `${percent}%` }} /></i>
          <small>{percent}%</small>
        </span>
        <span className="mobile-only mobile-token-count">{t('{count} tokens', { count: contextPack.token_count.toLocaleString(locale) })}</span>
        {expanded ? <ChevronUp size={17} /> : <ChevronDown size={17} />}
      </button>

      {expanded ? (
        <>
          {contextPack.coverage.status !== 'complete' ? (
            <div className="pack-coverage-warning" role="status">
              <AlertTriangle size={14} />
              <span><strong>{t('{status} capture', { status: t(contextPack.coverage.status) })}</strong>{[...contextPack.coverage.missing, ...contextPack.coverage.warnings].map((message) => t(message)).join(' · ')}</span>
            </div>
          ) : null}
          {temporal ? (
            <div className={`pack-temporal-summary temporal-${temporal.status}`}>
              <GitCompareArrows size={15} />
              <strong>{t(temporalLabel(temporal.status))}</strong>
              <code>{shortSha(temporal.baseline_head ?? checkpoint.sha, t('Unknown'))}</code>
              <span aria-hidden="true">→</span>
              <code>{shortSha(temporal.current_head ?? temporal.baseline_head ?? checkpoint.sha, t('Unknown'))}</code>
            </div>
          ) : null}
          <div className="context-pack-content">
            <PackSection title="Then" count={t('{count} items', { count: contextPack.facts.length + (contextPack.goal ? 1 : 0) })}>
              {contextPack.goal ? <PackText label="goal" text={contextPack.goal} /> : <p>{t('No verified goal was selected.')}</p>}
              {contextPack.facts.map((fact) => <PackText key={fact.id} label={fact.kind} text={fact.content} />)}
            </PackSection>
            <PackSection title="Since" count={t('{count} files', { count: changes.length || contextPack.files.length })}>
              {changes.length > 0
                ? changes.map((change) => <PackText key={`${change.previousPath ?? ''}-${change.path}-${change.status}`} label={change.status} text={changeText(change)} />)
                : contextPack.files.map((file) => <PackText key={`${file.path}-${file.status}`} label={file.status} text={file.path} />)}
              {changes.length === 0 && contextPack.files.length === 0 ? <p>{t('No relevant file changes were selected.')}</p> : null}
            </PackSection>
            <PackSection title="Now" count={t('{count} tests', { count: contextPack.tests.length })}>
              {contextPack.current_validation ? (
                <PackText
                  label={contextPack.current_validation.status}
                  text={t('{count} verified paths at {sha}', { count: (contextPack.current_validation.verified_paths ?? []).length, sha: shortSha(contextPack.current_validation.current_head, t('Unknown')) })}
                />
              ) : null}
              {contextPack.tests.map((test) => <PackText key={`${test.name}-${test.status}`} icon={<CheckCircle2 size={13} className={test.status === 'passed' ? 'success-text' : 'warning'} />} label={test.status} text={test.name} />)}
              {!contextPack.current_validation && contextPack.tests.length === 0 ? <p>{t('No current validation result was selected.')}</p> : null}
            </PackSection>
            <PackSection title="Needs review" count={t('{count} items', { count: reviewCount })}>
              {temporal && temporal.status !== 'unchanged' ? <PackText label={temporal.status} text={t(temporalLabel(temporal.status))} /> : null}
              {contextPack.unresolved_items.map((fact) => <PackText key={fact.id} label="open" text={fact.content} />)}
              {warnings.map((warning, index) => <PackText key={`${warning}-${index}`} label="warning" text={t(warning)} />)}
              {reviewCount === 0 ? <p>{t('No unresolved or stale items were selected.')}</p> : null}
            </PackSection>
          </div>
        </>
      ) : null}
    </section>
  );
}

function PackSection({ title, count, children }: { title: string; count: string; children: React.ReactNode }) {
  const { t } = useI18n();
  return (
    <div className="pack-section">
      <header><strong>{t(title)}</strong><span>{count}</span></header>
      <div>{children}</div>
    </div>
  );
}

function PackText({ icon, label, text }: { icon?: React.ReactNode; label: string; text: string }) {
  const { t } = useI18n();
  return <span className="pack-text">{icon}<small>{t(label.replaceAll('_', ' '))}</small><span>{text}</span></span>;
}
