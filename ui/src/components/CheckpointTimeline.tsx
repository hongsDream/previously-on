import {
  AlertTriangle,
  Check,
  ChevronRight,
  CircleHelp,
  Clock3,
  FileText,
  GitBranch,
  MessageSquareText,
} from 'lucide-react';
import { useEffect, useState } from 'react';
import type { Checkpoint, RelatedChange, TemporalStatus } from '../types';
import { FreshnessBadge } from './StatusBadge';
import { useI18n } from '../i18n-context';

interface CheckpointTimelineProps {
  checkpoints: Checkpoint[];
  selectedId: string;
  onSelect: (checkpoint: Checkpoint) => void;
}

const timeUnits = [
  ['year', 365 * 24 * 60 * 60],
  ['month', 30 * 24 * 60 * 60],
  ['week', 7 * 24 * 60 * 60],
  ['day', 24 * 60 * 60],
  ['hour', 60 * 60],
  ['minute', 60],
] as const;

function formatRelativeAge(value: string, locale: string, unknown: string, justNow: string, now = Date.now()) {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return unknown;
  const relativeFormatter = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  const differenceSeconds = (timestamp - now) / 1_000;
  for (const [unit, seconds] of timeUnits) {
    if (Math.abs(differenceSeconds) >= seconds) {
      return relativeFormatter.format(Math.round(differenceSeconds / seconds), unit);
    }
  }
  return justNow;
}

function contextUtilization(checkpoint: Checkpoint) {
  const usage = checkpoint.contextUsage;
  if (!usage || usage.modelContextWindow <= 0) return null;
  return Math.min(100, Math.round((usage.totalTokens / usage.modelContextWindow) * 100));
}

function shortSha(value: string | undefined, unknown: string) {
  return value ? value.slice(0, 8) : unknown;
}

function formatChange(change: RelatedChange, translate: (message: string) => string) {
  return change.status === 'renamed' && change.previousPath
    ? `${change.previousPath} → ${change.path}`
    : `${translate(change.status)}: ${change.path}`;
}

function sessionTitle(
  title: string,
  translate: (message: string, values?: Record<string, string | number>) => string,
) {
  const match = /^Session\s+(.+)$/.exec(title);
  return match ? translate('Session {value}', { value: match[1] }) : title;
}

function TemporalBadge({ status }: { status: TemporalStatus }) {
  const { t } = useI18n();
  return <span className={`temporal-badge temporal-${status}`}>{t(status.replaceAll('_', ' '))}</span>;
}

export function CheckpointTimeline({ checkpoints, selectedId, onSelect }: CheckpointTimelineProps) {
  const { t, locale } = useI18n();
  const [now, setNow] = useState(() => Date.now());
  const dateFormatter = new Intl.DateTimeFormat(locale, {
    month: 'short', day: 'numeric', year: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false,
  });
  const relativeAge = (value: string) => formatRelativeAge(value, locale, t('Unknown activity'), t('Just now'), now);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  return (
    <section className="timeline" aria-labelledby="timeline-title">
      <h2 id="timeline-title" className="sr-only">{t('Session checkpoints')}</h2>
      <div className="timeline-columns desktop-only" aria-hidden="true">
        <span>{t('Session / Activity')}</span>
        <span>{t('Git position')}</span>
        <span>{t('Changes')}</span>
        <span>{t('Tests')}</span>
        <span>{t('Context')}</span>
      </div>
      <ol>
        {checkpoints.map((checkpoint) => {
          const selected = checkpoint.id === selectedId;
          const lastActivityAt = checkpoint.lastActivityAt ?? checkpoint.capturedAt;
          const utilization = contextUtilization(checkpoint);
          const temporal = checkpoint.temporalRevalidation;
          const baselineSha = temporal?.baselineSha ?? checkpoint.sha;
          const currentSha = temporal?.currentSha;
          const changes = temporal?.changes ?? [];
          const continuationRecommended = checkpoint.continuationState === 'eligible'
            || checkpoint.continuationState === 'suggested'
            || checkpoint.continuationAdvice?.action === 'new_thread';
          return (
            <li key={checkpoint.id} className={selected ? 'selected' : ''}>
              <span className="timeline-line" aria-hidden="true" />
              <button
                className="checkpoint-marker"
                type="button"
                aria-label={t('Select checkpoint {sequence}', { sequence: checkpoint.sequence })}
                aria-current={selected ? 'step' : undefined}
                onClick={() => onSelect(checkpoint)}
              >
                <span className="desktop-marker-dot" />
                <span className="mobile-marker-number">{checkpoint.sequence}</span>
              </button>
              <button className="checkpoint-row" type="button" onClick={() => onSelect(checkpoint)}>
                <div className="checkpoint-session">
                  <div className="desktop-only session-title-line">
                    <strong>{checkpoint.sequence}</strong>
                    <span>{sessionTitle(checkpoint.sessionTitle, t)}</span>
                  </div>
                  <div className="mobile-only mobile-checkpoint-topline">
                    <span>{relativeAge(lastActivityAt)}</span>
                    <span><small>SHA</small> <code>{shortSha(currentSha ?? baselineSha, t('Unknown')).slice(0, 7)}</code></span>
                    {temporal ? <TemporalBadge status={temporal.status} /> : <FreshnessBadge status={checkpoint.freshness} />}
                    <ChevronRight size={18} />
                  </div>
                  <span className={`session-state desktop-only ${continuationRecommended ? 'continuation-warning' : ''}`}>
                    {continuationRecommended ? <AlertTriangle size={12} /> : checkpoint.state === 'confirmed' ? <Check size={12} /> : <span className="hollow-dot" />}
                    {continuationRecommended ? t('New thread suggested') : t('Checkpoint · {state}', { state: checkpoint.state === 'confirmed' ? t('Confirmed decision') : t('Draft') })}
                  </span>
                  <small className="captured desktop-only">
                    <Clock3 size={11} />
                    {dateFormatter.format(new Date(lastActivityAt))} · {relativeAge(lastActivityAt)}
                  </small>
                  <span className="session-counters desktop-only">
                    {checkpoint.turnCount !== undefined ? <small>{t('{count} turns', { count: checkpoint.turnCount })}</small> : null}
                    {checkpoint.compactionCount !== undefined ? <small>{t('{count} compactions', { count: checkpoint.compactionCount })}</small> : null}
                    {checkpoint.sourceThreadId ? <small title={checkpoint.sourceThreadId}>{t('Task {id}', { id: checkpoint.sourceThreadId.slice(0, 8) })}</small> : null}
                  </span>
                  <div className="mobile-only mobile-checkpoint-stats">
                    <span><FileText size={17} /> {t('{count} files', { count: checkpoint.filesChanged })}</span>
                    <span><MessageSquareText size={17} /> {t('{count} compactions', { count: checkpoint.compactionCount ?? 0 })}</span>
                    <span className={selected ? 'accent-stat' : ''}>{utilization === null ? t('{percent}% capture', { percent: checkpoint.coverage }) : t('{percent}% context', { percent: utilization })}</span>
                  </div>
                </div>

                <div className="checkpoint-branch desktop-only">
                  <span><GitBranch size={13} /> {checkpoint.branch}</span>
                  <div className="sha-line">
                    <code>{shortSha(baselineSha, t('Unknown'))}</code>
                    {currentSha && currentSha !== baselineSha ? <><span aria-hidden="true">→</span><code>{shortSha(currentSha, t('Unknown'))}</code></> : null}
                  </div>
                  {temporal ? <TemporalBadge status={temporal.status} /> : <FreshnessBadge status={checkpoint.freshness} />}
                </div>
                <div className="metric desktop-only">
                  <strong>{t('{count} files', { count: checkpoint.filesChanged })}</strong>
                  <span><em>+{checkpoint.additions}</em> <b>−{checkpoint.deletions}</b></span>
                  {changes.length > 0 ? (
                    <small className="change-paths" title={changes.map((change) => formatChange(change, t)).join('\n')}>
                      {formatChange(changes[0], t)}{changes.length > 1 ? ` +${changes.length - 1}` : ''}
                    </small>
                  ) : <small>{t('No revalidated delta')}</small>}
                </div>
                <div className="metric tests desktop-only">
                  <strong>{checkpoint.testsFailed === 0 ? <Check size={12} /> : <CircleHelp size={12} />} {t('{count} passed', { count: checkpoint.testsPassed })}</strong>
                  <span className={checkpoint.testsFailed > 0 ? 'negative' : ''}>{t('{count} failed', { count: checkpoint.testsFailed })}</span>
                  <small>{checkpoint.turnCount === undefined ? t('Turns unavailable') : t('{count} turns', { count: checkpoint.turnCount })}</small>
                </div>
                <div className="metric context-metric desktop-only">
                  <strong>{utilization === null ? t('{percent}% capture', { percent: checkpoint.coverage }) : t('{percent}% used', { percent: utilization })}</strong>
                  <span>{checkpoint.compactionCount === undefined ? t('Compactions unavailable') : t('{count} compactions', { count: checkpoint.compactionCount })}</span>
                  <small>{checkpoint.contextUsage ? t('{used} / {total} tokens', { used: checkpoint.contextUsage.totalTokens.toLocaleString(locale), total: checkpoint.contextUsage.modelContextWindow.toLocaleString(locale) }) : t('Token usage unavailable')}</small>
                </div>
                <ChevronRight className="row-chevron desktop-only" size={19} />
              </button>
            </li>
          );
        })}
      </ol>
      <button className="secondary-button load-more desktop-only" type="button">{t('Load more sessions')}</button>
    </section>
  );
}
