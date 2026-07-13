import { History, Sparkles, X } from 'lucide-react';
import type { ResumeCandidate, Task } from '../types';

interface ResumeBannerProps {
  candidate: ResumeCandidate;
  task: Task;
  onReview: () => void;
  onDismiss: () => void;
}

export function ResumeBanner({ candidate, task, onReview, onDismiss }: ResumeBannerProps) {
  return (
    <section className="resume-banner" aria-label="Resume here?">
      <span className="resume-icon desktop-only"><History size={18} /></span>
      <span className="resume-icon mobile-only"><Sparkles size={21} /></span>
      <div className="resume-copy">
        <strong id="resume-title" className="mobile-only">Resume here?</strong>
        <span className="desktop-resume-copy">You have {candidate.uncompletedSessions} uncompleted sessions on this task.<br />Resume “{task.title}”?</span>
        <span className="mobile-only">{candidate.reason}</span>
      </div>
      <div className="resume-actions">
        <button className="primary-button" type="button" onClick={onReview}>Review</button>
        <button className="secondary-button" type="button" onClick={onDismiss}>Dismiss</button>
        <button className="icon-button desktop-only" type="button" aria-label="Dismiss resume suggestion" onClick={onDismiss}><X size={16} /></button>
      </div>
    </section>
  );
}
