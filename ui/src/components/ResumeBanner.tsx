import { History, Sparkles, X } from 'lucide-react';
import type { ResumeCandidate, Task } from '../types';
import { useI18n } from '../i18n-context';

interface ResumeBannerProps {
  candidate: ResumeCandidate;
  task: Task;
  onReview: () => void;
  onDismiss: () => void;
}

export function ResumeBanner({ candidate, task, onReview, onDismiss }: ResumeBannerProps) {
  const { t } = useI18n();
  return (
    <section className="resume-banner" aria-label={t('Resume here?')}>
      <span className="resume-icon desktop-only"><History size={18} /></span>
      <span className="resume-icon mobile-only"><Sparkles size={21} /></span>
      <div className="resume-copy">
        <strong id="resume-title" className="mobile-only">{t('Resume here?')}</strong>
        <span className="desktop-resume-copy">{t('You have {count} uncompleted sessions on this task.', { count: candidate.uncompletedSessions })}<br />{t('Resume “{title}”?', { title: task.title })}</span>
        <span className="mobile-only">{candidate.reason}</span>
      </div>
      <div className="resume-actions">
        <button className="primary-button" type="button" onClick={onReview}>{t('Review')}</button>
        <button className="secondary-button" type="button" onClick={onDismiss}>{t('Dismiss')}</button>
        <button className="icon-button desktop-only" type="button" aria-label={t('Dismiss resume suggestion')} onClick={onDismiss}><X size={16} /></button>
      </div>
    </section>
  );
}
