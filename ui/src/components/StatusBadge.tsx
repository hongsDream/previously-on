import type { FactStatus, Freshness } from '../types';

export function FreshnessBadge({ status }: { status: Freshness }) {
  return <span className={`status-badge status-${status}`}>{status[0].toUpperCase() + status.slice(1)}</span>;
}

export function FactBadge({ status }: { status: FactStatus }) {
  return <span className={`fact-badge fact-${status}`}>{status[0].toUpperCase() + status.slice(1)} decision</span>;
}
