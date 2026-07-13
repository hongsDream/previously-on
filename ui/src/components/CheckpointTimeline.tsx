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

interface CheckpointTimelineProps {
  checkpoints: Checkpoint[];
  selectedId: string;
  onSelect: (checkpoint: Checkpoint) => void;
}

const dateFormatter = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hour12: false,
});

const relativeFormatter = new Intl.RelativeTimeFormat('en-US', { numeric: 'auto' });
const timeUnits = [
  ['year', 365 * 24 * 60 * 60],
  ['month', 30 * 24 * 60 * 60],
  ['week', 7 * 24 * 60 * 60],
  ['day', 24 * 60 * 60],
  ['hour', 60 * 60],
  ['minute', 60],
] as const;

function formatRelativeAge(value: string, now = Date.now()) {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return 'Unknown activity';
  const differenceSeconds = (timestamp - now) / 1_000;
  for (const [unit, seconds] of timeUnits) {
    if (Math.abs(differenceSeconds) >= seconds) {
      return relativeFormatter.format(Math.round(differenceSeconds / seconds), unit);
    }
  }
  return 'just now';
}

function contextUtilization(checkpoint: Checkpoint) {
  const usage = checkpoint.contextUsage;
  if (!usage || usage.modelContextWindow <= 0) return null;
  return Math.min(100, Math.round((usage.totalTokens / usage.modelContextWindow) * 100));
}

function shortSha(value: string | undefined) {
  return value ? value.slice(0, 8) : 'unknown';
}

function formatChange(change: RelatedChange) {
  return change.status === 'renamed' && change.previousPath
    ? `${change.previousPath} → ${change.path}`
    : `${change.status}: ${change.path}`;
}

function TemporalBadge({ status }: { status: TemporalStatus }) {
  return <span className={`temporal-badge temporal-${status}`}>{status.replaceAll('_', ' ')}</span>;
}

export function CheckpointTimeline({ checkpoints, selectedId, onSelect }: CheckpointTimelineProps) {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => window.clearInterval(timer);
  }, []);

  return (
    <section className="timeline" aria-labelledby="timeline-title">
      <h2 id="timeline-title" className="sr-only">Session checkpoints</h2>
      <div className="timeline-columns desktop-only" aria-hidden="true">
        <span>Session / Activity</span>
        <span>Git position</span>
        <span>Changes</span>
        <span>Tests</span>
        <span>Context</span>
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
                aria-label={`Select checkpoint ${checkpoint.sequence}`}
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
                    <span>{checkpoint.sessionTitle}</span>
                  </div>
                  <div className="mobile-only mobile-checkpoint-topline">
                    <span>{formatRelativeAge(lastActivityAt, now)}</span>
                    <span><small>SHA</small> <code>{shortSha(currentSha ?? baselineSha).slice(0, 7)}</code></span>
                    {temporal ? <TemporalBadge status={temporal.status} /> : <FreshnessBadge status={checkpoint.freshness} />}
                    <ChevronRight size={18} />
                  </div>
                  <span className={`session-state desktop-only ${continuationRecommended ? 'continuation-warning' : ''}`}>
                    {continuationRecommended ? <AlertTriangle size={12} /> : checkpoint.state === 'confirmed' ? <Check size={12} /> : <span className="hollow-dot" />}
                    {continuationRecommended ? 'New thread suggested' : `Checkpoint · ${checkpoint.state === 'confirmed' ? 'Confirmed decision' : 'Draft'}`}
                  </span>
                  <small className="captured desktop-only">
                    <Clock3 size={11} />
                    {dateFormatter.format(new Date(lastActivityAt))} · {formatRelativeAge(lastActivityAt, now)}
                  </small>
                  <span className="session-counters desktop-only">
                    {checkpoint.turnCount !== undefined ? <small>{checkpoint.turnCount} turns</small> : null}
                    {checkpoint.compactionCount !== undefined ? <small>{checkpoint.compactionCount} compactions</small> : null}
                    {checkpoint.sourceThreadId ? <small title={checkpoint.sourceThreadId}>Thread {checkpoint.sourceThreadId.slice(0, 8)}</small> : null}
                  </span>
                  <div className="mobile-only mobile-checkpoint-stats">
                    <span><FileText size={17} /> {checkpoint.filesChanged} files</span>
                    <span><MessageSquareText size={17} /> {checkpoint.compactionCount ?? 0} compact</span>
                    <span className={selected ? 'accent-stat' : ''}>{utilization === null ? `${checkpoint.coverage}% capture` : `${utilization}% context`}</span>
                  </div>
                </div>

                <div className="checkpoint-branch desktop-only">
                  <span><GitBranch size={13} /> {checkpoint.branch}</span>
                  <div className="sha-line">
                    <code>{shortSha(baselineSha)}</code>
                    {currentSha && currentSha !== baselineSha ? <><span aria-hidden="true">→</span><code>{shortSha(currentSha)}</code></> : null}
                  </div>
                  {temporal ? <TemporalBadge status={temporal.status} /> : <FreshnessBadge status={checkpoint.freshness} />}
                </div>
                <div className="metric desktop-only">
                  <strong>{checkpoint.filesChanged} files</strong>
                  <span><em>+{checkpoint.additions}</em> <b>−{checkpoint.deletions}</b></span>
                  {changes.length > 0 ? (
                    <small className="change-paths" title={changes.map(formatChange).join('\n')}>
                      {formatChange(changes[0])}{changes.length > 1 ? ` +${changes.length - 1}` : ''}
                    </small>
                  ) : <small>No revalidated delta</small>}
                </div>
                <div className="metric tests desktop-only">
                  <strong>{checkpoint.testsFailed === 0 ? <Check size={12} /> : <CircleHelp size={12} />} {checkpoint.testsPassed} passed</strong>
                  <span className={checkpoint.testsFailed > 0 ? 'negative' : ''}>{checkpoint.testsFailed} failed</span>
                  <small>{checkpoint.turnCount === undefined ? 'Turns unavailable' : `${checkpoint.turnCount} turns`}</small>
                </div>
                <div className="metric context-metric desktop-only">
                  <strong>{utilization === null ? `${checkpoint.coverage}% capture` : `${utilization}% used`}</strong>
                  <span>{checkpoint.compactionCount === undefined ? 'Compactions unavailable' : `${checkpoint.compactionCount} compactions`}</span>
                  <small>{checkpoint.contextUsage ? `${checkpoint.contextUsage.totalTokens.toLocaleString()} / ${checkpoint.contextUsage.modelContextWindow.toLocaleString()} tokens` : 'Token usage unavailable'}</small>
                </div>
                <ChevronRight className="row-chevron desktop-only" size={19} />
              </button>
            </li>
          );
        })}
      </ol>
      <button className="secondary-button load-more desktop-only" type="button">Load more sessions</button>
    </section>
  );
}
