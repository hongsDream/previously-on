import { AlertTriangle, CheckCircle2, Clock3 } from 'lucide-react';
import { useI18n } from '../i18n-context';
import type { CodexImportReportV1 } from '../types';

export function CodexSyncStatus({ report }: { report?: CodexImportReportV1 }) {
  const { locale, t } = useI18n();
  if (!report) return null;
  const complete = report.status === 'complete';
  return (
    <section className={`codex-sync-status sync-${report.status}`} role="status" aria-label={t('Codex app history synchronization')}>
      <div className="codex-sync-summary">
        {complete ? <CheckCircle2 size={16} /> : <AlertTriangle size={16} />}
        <strong>{t(complete ? 'Synchronization complete' : report.status === 'unsupported' ? 'App Server unsupported' : 'Synchronization degraded')}</strong>
        <span>{t('{count} tasks imported', { count: report.importedTaskCount })}</span>
        <span>{t('{count} duplicates', { count: report.duplicateCount })}</span>
        <span>{t('{count} missing or unknown items', { count: report.missingOrUnknownItems.length })}</span>
        <span><Clock3 size={13} /> {new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(report.lastSyncedAt))}</span>
      </div>
      {report.technicalDetails.length > 0 ? (
        <details>
          <summary>{t('Technical details')}</summary>
          <ul>{report.technicalDetails.map((detail) => <li key={detail}>{detail}</li>)}</ul>
        </details>
      ) : null}
    </section>
  );
}
