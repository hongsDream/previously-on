import { Check, CircleAlert, LoaderCircle, RefreshCw, ShieldCheck } from 'lucide-react';
import { useState } from 'react';
import { setupCodex, toUiError, type SetupCodexResponse, type UiError } from '../lib/api';
import { useI18n } from '../i18n-context';
import { ErrorNotice } from './ErrorNotice';

const PATH_PLACEHOLDER = '/absolute/path/to/repository';

export function FirstRunSetup({ refreshPending, onRefresh }: {
  refreshPending: boolean;
  onRefresh: () => void;
}) {
  const { t } = useI18n();
  const [repositoryPath, setRepositoryPath] = useState('');
  const [confirmed, setConfirmed] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [setupError, setSetupError] = useState<UiError | null>(null);
  const [result, setResult] = useState<SetupCodexResponse | null>(null);
  const normalizedPath = repositoryPath.trim();
  const pathIsAbsolute = normalizedPath.startsWith('/');

  const connect = async () => {
    if (!confirmed || !pathIsAbsolute || connecting) return;
    setConnecting(true);
    setSetupError(null);
    try {
      setResult(await setupCodex(normalizedPath));
    } catch (error) {
      setSetupError(toUiError(error, 'Codex could not be connected.'));
    } finally {
      setConnecting(false);
    }
  };

  if (result) {
    const failedChecks = result.doctor.checks.filter((check) => !check.ok);
    return (
      <section className="first-run-setup setup-complete" aria-labelledby="first-run-title">
        <span className="setup-success-mark" aria-hidden="true"><Check size={24} /></span>
        <h1 id="first-run-title">{t('Codex connection installed')}</h1>
        <p>{t('PreviouslyOn registered {path}, backed up existing Codex configuration, and ran the local integration checks.', { path: result.repositoryPath })}</p>

        <div className={`setup-doctor-summary ${result.doctor.healthy ? 'healthy' : 'needs-attention'}`} role="status">
          {result.doctor.healthy ? <ShieldCheck size={19} /> : <CircleAlert size={19} />}
          <span>
            <strong>{result.doctor.healthy ? t('Local checks passed') : t('Setup finished with checks to review')}</strong>
            <small>{result.doctor.healthy
              ? t('{count} setup and capability checks passed.', { count: result.doctor.checks.length })
              : t('{count} checks need attention. You can run previously doctor after restarting Codex.', { count: failedChecks.length })}</small>
          </span>
        </div>

        {failedChecks.length > 0 ? (
          <ul className="setup-doctor-failures" aria-label={t('Checks needing attention')}>
            {failedChecks.map((check) => <li key={check.name}><strong>{check.name}</strong><span>{check.detail}</span></li>)}
          </ul>
        ) : null}

        <div className="restart-instruction">
          <span className="setup-step-number">1</span>
          <span><strong>{t('Restart Codex once')}</strong><small>{t('Quit Codex completely and reopen it so the managed Hooks and MCP server are loaded.')}</small></span>
        </div>
        <button className="primary-button setup-connect-button" type="button" disabled={refreshPending} onClick={onRefresh}>
          {refreshPending ? <LoaderCircle className="spin-icon" size={16} /> : <RefreshCw size={16} />}
          {refreshPending ? t('Checking connection…') : t('I restarted Codex · Continue')}
        </button>
      </section>
    );
  }

  return (
    <section className="first-run-setup" aria-labelledby="first-run-title">
      <span className="empty-lineage-mark" aria-hidden="true" />
      <h1 id="first-run-title">{t('Connect Codex to a project')}</h1>
      <p>{t('Connect a local Git project. PreviouslyOn keeps every registered project separate and lets you switch or review all projects from this device.')}</p>

      <label htmlFor="repository-path">{t('Repository path')}</label>
      <input
        id="repository-path"
        type="text"
        value={repositoryPath}
        placeholder={PATH_PLACEHOLDER}
        autoComplete="off"
        spellCheck={false}
        onChange={(event) => {
          setRepositoryPath(event.target.value);
          setSetupError(null);
        }}
        aria-describedby="repository-path-help"
      />
      <small id="repository-path-help" className={normalizedPath && !pathIsAbsolute ? 'field-error' : 'field-help'}>
        {normalizedPath && !pathIsAbsolute ? t('Enter an absolute path beginning with /.') : t('Use the absolute path to a Git worktree. Paths containing spaces are supported.')}
      </small>

      <div className="setup-change-preview" aria-label={t('Local changes')}>
        <h2>{t('What will change')}</h2>
        <ul>
          <li><ShieldCheck size={17} /><span><strong>{t('Codex Hooks and MCP')}</strong><small>{t('Managed entries are added to your local Codex configuration.')}</small></span></li>
          <li><ShieldCheck size={17} /><span><strong>{t('Recoverable backups')}</strong><small>{t('Existing configuration is backed up before any managed file is replaced.')}</small></span></li>
          <li><ShieldCheck size={17} /><span><strong>{t('Local-only verification')}</strong><small>{t('Doctor checks run without creating a task, calling a model, or uploading telemetry.')}</small></span></li>
        </ul>
      </div>

      <label className="setup-consent">
        <input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)} />
        <span>{t('I approve updating my local Codex configuration for this repository.')}</span>
      </label>

      <button
        className="primary-button setup-connect-button"
        type="button"
        disabled={!pathIsAbsolute || !confirmed || connecting}
        onClick={() => void connect()}
      >
        {connecting ? <LoaderCircle className="spin-icon" size={16} /> : <ShieldCheck size={16} />}
        {connecting ? t('Connecting and checking…') : t('Connect Codex')}
      </button>
      {setupError ? <ErrorNotice error={setupError} className="setup-copy-error" /> : null}
    </section>
  );
}

export function RegisteredEmptyActions({ repositoryPath, refreshPending, onRefresh }: {
  repositoryPath: string;
  refreshPending: boolean;
  onRefresh: () => void;
}) {
  const { t } = useI18n();
  const path = repositoryPath.trim() || PATH_PLACEHOLDER;
  const steps = [
    {
      title: t('Work in Codex Desktop'),
      description: t('Open {path} in Codex Desktop and complete a task normally.', { path }),
    },
    {
      title: t('Import from this device'),
      description: t('Return here and choose Sync Codex app history. The import starts only when you request it and stays local.'),
    },
  ];

  return (
    <div className="registered-empty-actions">
      <ol className="setup-command-list">
        {steps.map((step, index) => (
          <li key={step.title}>
            <span className="setup-step-number">{index + 1}</span>
            <span><strong>{step.title}</strong><small>{step.description}</small></span>
          </li>
        ))}
      </ol>
      <button className="secondary-button refresh-bootstrap-button" type="button" disabled={refreshPending} onClick={onRefresh}>
        <RefreshCw size={15} className={refreshPending ? 'spin-icon' : ''} />
        {refreshPending ? t('Refreshing status…') : t('Refresh status')}
      </button>
    </div>
  );
}
