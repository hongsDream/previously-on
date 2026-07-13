import { Check, ChevronRight, CircleHelp, Clipboard, FileText, FlaskConical, GitBranch, PieChart } from 'lucide-react';
import type { Checkpoint } from '../types';
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

const shortDateFormatter = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  hour12: false,
});

export function CheckpointTimeline({ checkpoints, selectedId, onSelect }: CheckpointTimelineProps) {
  return (
    <section className="timeline" aria-labelledby="timeline-title">
      <h2 id="timeline-title" className="sr-only">Session checkpoints</h2>
      <div className="timeline-columns desktop-only" aria-hidden="true">
        <span>Session / Checkpoint</span>
        <span>Branch / SHA</span>
        <span>Changes</span>
        <span>Tests</span>
        <span>Capture</span>
      </div>
      <ol>
        {checkpoints.map((checkpoint) => {
          const selected = checkpoint.id === selectedId;
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
                    <span>{shortDateFormatter.format(new Date(checkpoint.capturedAt))}</span>
                    <span><small>SHA</small> <code>{checkpoint.sha.slice(0, 7)}</code></span>
                    <FreshnessBadge status={checkpoint.freshness} />
                    <ChevronRight size={18} />
                  </div>
                  <span className="session-state desktop-only">
                    {checkpoint.state === 'confirmed' ? <Check size={12} /> : <span className="hollow-dot" />}
                    Checkpoint · {checkpoint.state === 'confirmed' ? 'Confirmed decision' : 'Draft'}
                  </span>
                  <small className="captured desktop-only">{dateFormatter.format(new Date(checkpoint.capturedAt))}</small>
                  <div className="mobile-only mobile-checkpoint-stats">
                    <span><FileText size={17} /> {checkpoint.filesChanged} files</span>
                    <span><FlaskConical size={17} /> {checkpoint.testsPassed + checkpoint.testsFailed} tests</span>
                    <span className={selected ? 'accent-stat' : ''}><PieChart size={17} /> {checkpoint.coverage}% capture</span>
                  </div>
                </div>

                <div className="checkpoint-branch desktop-only">
                  <span><GitBranch size={13} /> {checkpoint.branch}</span>
                  <code>{checkpoint.sha.slice(0, 8)}</code>
                  <Clipboard size={13} />
                </div>
                <div className="metric desktop-only">
                  <strong>{checkpoint.filesChanged} files</strong>
                  <span><em>+{checkpoint.additions}</em> <b>−{checkpoint.deletions}</b></span>
                  <small>View</small>
                </div>
                <div className="metric tests desktop-only">
                  <strong>{checkpoint.testsFailed === 0 ? <Check size={12} /> : <CircleHelp size={12} />} {checkpoint.testsPassed} passed</strong>
                  <span className={checkpoint.testsFailed > 0 ? 'negative' : ''}>{checkpoint.testsFailed} failed</span>
                  <small>View</small>
                </div>
                <div className="metric coverage desktop-only">
                  <strong><PieChart size={14} /> {checkpoint.coverage}%</strong>
                  <span className={checkpoint.coverageDelta >= 0 ? 'positive' : 'negative'}>{checkpoint.coverageDelta >= 0 ? '+' : ''}{checkpoint.coverageDelta}%</span>
                  <small>View</small>
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
