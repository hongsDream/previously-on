import { useI18n } from '../i18n-context';
import type { UiError } from '../lib/api';

export function ErrorNotice({ error, className = 'action-error' }: {
  error: UiError;
  className?: string;
}) {
  const { t } = useI18n();
  return (
    <div className={className} role="alert">
      <span>{t(error.messageKey)}</span>
      {error.technicalDetails.length > 0 ? (
        <details>
          <summary>{t('Technical details')}</summary>
          <ul>{error.technicalDetails.map((detail) => <li key={detail}>{detail}</li>)}</ul>
        </details>
      ) : null}
    </div>
  );
}
