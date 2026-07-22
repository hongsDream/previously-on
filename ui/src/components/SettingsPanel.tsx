import { Bot, CircleAlert, CircleCheck, LockKeyhole, ShieldCheck } from 'lucide-react';
import type { AiRefreshCapabilityV1 } from '../types';
import { useI18n } from '../i18n-context';

interface SettingsPanelProps {
  capability: AiRefreshCapabilityV1;
}

const statusCopy: Record<AiRefreshCapabilityV1['status'], { title: string; description: string }> = {
  ready: {
    title: 'Ready for explicit refresh',
    description: 'The managed input-only permission profile was found and its effective requirements were verified.',
  },
  needs_setup: {
    title: 'Setup required',
    description: 'AI fact refresh was not enabled during setup. Re-run setup with the opt-in flag before using it.',
  },
  unsupported: {
    title: 'App Server unsupported',
    description: 'This local App Server cannot verify the named permission profile contract required for safe refresh.',
  },
  blocked: {
    title: 'Refresh blocked',
    description: 'PreviouslyOn could not verify every effective permission requirement, so refresh remains disabled.',
  },
};

export function SettingsPanel({ capability }: SettingsPanelProps) {
  const { language, locale, setLanguage, t } = useI18n();
  const copy = statusCopy[capability.status];
  const StatusIcon = capability.status === 'ready' ? CircleCheck : CircleAlert;
  return (
    <main className="settings-workspace" aria-labelledby="settings-title">
      <header className="settings-hero">
        <span>{t('Local capabilities')}</span>
        <h1 id="settings-title">{t('Settings')}</h1>
        <p>{t('PreviouslyOn verifies local capabilities before exposing optional actions. History review remains local and available without AI refresh.')}</p>
      </header>

      <section className="settings-panel language-settings" aria-labelledby="language-settings-title">
        <header>
          <span className="settings-icon" aria-hidden="true">가</span>
          <div>
            <h2 id="language-settings-title">{t('Interface language')}</h2>
            <p>{t('Saved on this browser')}</p>
          </div>
        </header>
        <div className="language-settings-body">
          <label htmlFor="interface-language">{t('Language')}</label>
          <select id="interface-language" value={language} onChange={(event) => setLanguage(event.target.value as 'en' | 'ko')}>
            <option value="ko">{t('Korean')}</option>
            <option value="en">{t('English')}</option>
          </select>
          <p>{t('Choose the language used for navigation, guidance, and local status messages. Repository content and commands are never translated.')}</p>
        </div>
      </section>

      <section className="settings-panel" aria-labelledby="ai-refresh-settings-title">
        <header>
          <span className="settings-icon"><Bot size={20} /></span>
          <div>
            <h2 id="ai-refresh-settings-title">{t('AI-assisted fact refresh')}</h2>
            <p>{t('Beta · explicit opt-in · user initiated only')}</p>
          </div>
          <span className={`capability-status capability-${capability.status}`}><StatusIcon size={13} /> {t(capability.status.replace('_', ' '))}</span>
        </header>
        <div className="settings-capability-body">
          <div className="capability-summary">
            <strong>{t(copy.title)}</strong>
            <p>{t(copy.description)}</p>
          </div>
          <dl>
            <div><dt>{t('Permission profile')}</dt><dd><code>{capability.profileName || t('Unavailable')}</code></dd></div>
            <div><dt>{t('Network')}</dt><dd>{capability.status === 'ready' ? t('Disabled') : t('Not verified')}</dd></div>
            <div><dt>{t('Approval policy')}</dt><dd>{capability.status === 'ready' ? t('Never') : t('Not verified')}</dd></div>
            <div><dt>{t('Last verified')}</dt><dd>{formatCheckedAt(capability.checkedAt, locale, t('Unavailable'))}</dd></div>
          </dl>
          <ul className="capability-guardrails">
            <li><ShieldCheck size={15} /><span><strong>{t('Bounded verified input')}</strong>{t('Only redacted task facts, files, tests, and Regression Contracts can enter the refresh pack.')}</span></li>
            <li><LockKeyhole size={15} /><span><strong>{t('Candidate-only output')}</strong>{t('Model output never becomes Evidence. You must review it before it can become a Fact Candidate.')}</span></li>
          </ul>
          {capability.technicalDetails.length > 0 ? (
            <details>
              <summary>{t('Technical details')}</summary>
              <ul>{capability.technicalDetails.map((detail) => <li key={detail}>{detail}</li>)}</ul>
            </details>
          ) : null}
        </div>
      </section>
    </main>
  );
}

function formatCheckedAt(value: string | null | undefined, locale: string, unavailable: string) {
  if (!value) return unavailable;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? unavailable : date.toLocaleString(locale);
}
