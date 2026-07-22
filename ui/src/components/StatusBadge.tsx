import type { FactStatus, Freshness } from '../types';
import { useI18n } from '../i18n-context';

const freshnessCopy: Record<Freshness, string> = {
  fresh: 'Fresh',
  stale: 'Stale',
  broken: 'Broken',
};

const factCopy: Record<FactStatus, string> = {
  candidate: 'Candidate decision',
  confirmed: 'Confirmed decision',
  pinned: 'Pinned decision',
  invalid: 'Invalid decision',
  superseded: 'Superseded decision',
};

export function FreshnessBadge({ status }: { status: Freshness }) {
  const { t } = useI18n();
  return <span className={`status-badge status-${status}`}>{t(freshnessCopy[status])}</span>;
}

export function FactBadge({ status }: { status: FactStatus }) {
  const { t } = useI18n();
  return <span className={`fact-badge fact-${status}`}>{t(factCopy[status])}</span>;
}
