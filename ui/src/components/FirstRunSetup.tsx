import { Check, CircleAlert, Clipboard, LoaderCircle, RefreshCw, ShieldCheck, Terminal } from 'lucide-react';
import { useState } from 'react';
import { setupCodex, type SetupCodexResponse } from '../lib/api';
import { useI18n } from '../i18n-context';

const PATH_PLACEHOLDER = '/absolute/path/to/repository';

export function FirstRunSetup({ refreshPending, onRefresh }: {
  refreshPending: boolean;
  onRefresh: () => void;
}) {
  const { t } = useI18n();
  const [repositoryPath, setRepositoryPath] = useState('');
  const [confirmed, setConfirmed] = useState(false);
  const [connecting, setConnecting] = useState(false);
  const [setupError, setSetupError] = useState('');
  const [result, setResult] = useState<SetupCodexResponse | null>(null);
  const normalizedPath = repositoryPath.trim();
  const pathIsAbsolute = normalizedPath.startsWith('/');

  const connect = async () => {
    if (!confirmed || !pathIsAbsolute || connecting) return;
    setConnecting(true);
    setSetupError('');
    try {
      setResult(await setupCodex(normalizedPath));
    } catch (error) {
      setSetupError(error instanceof Error ? error.message : t('Codex could not be connected.'));
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
      <h1 id="first-run-title">{t('Connect Codex to your repository')}</h1>
      <p>{t('Choose one local Git repository for this pilot. PreviouslyOn will configure the local Codex integration and verify it here—no setup command is required.')}</p>

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
          setSetupError('');
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
      {setupError ? <p className="setup-copy-error" role="alert">{setupError}</p> : null}
    </section>
  );
}

export function RegisteredEmptyActions({ repositoryPath, refreshPending, onRefresh }: {
  repositoryPath: string;
  refreshPending: boolean;
  onRefresh: () => void;
}) {
  const { t } = useI18n();
  const [copied, setCopied] = useState<number | null>(null);
  const [copyError, setCopyError] = useState('');
  const path = shellArgument(repositoryPath.trim() || PATH_PLACEHOLDER);
  const commands = [
    { title: t('Start a captured Codex session'), value: `previously run codex --repo ${path} --` },
    { title: t('Check the local integration'), value: 'previously doctor' },
  ];

  const copy = async (value: string, index: number) => {
    setCopyError('');
    try {
      await navigator.clipboard.writeText(value);
      setCopied(index);
    } catch {
      setCopyError(t('Clipboard access was unavailable. Select and copy the command manually.'));
    }
  };

  return (
    <div className="registered-empty-actions">
      <ol className="setup-command-list">
        {commands.map((command, index) => (
          <li key={command.title}>
            <span className="setup-step-number">{index + 1}</span>
            <span><strong>{command.title}</strong><code><Terminal size={14} aria-hidden="true" />{command.value}</code></span>
            <button className="icon-button" type="button" aria-label={t('Copy {title}', { title: command.title })} onClick={() => void copy(command.value, index)}>
              {copied === index ? <Check size={16} /> : <Clipboard size={16} />}
            </button>
          </li>
        ))}
      </ol>
      <button className="secondary-button refresh-bootstrap-button" type="button" disabled={refreshPending} onClick={onRefresh}>
        <RefreshCw size={15} className={refreshPending ? 'spin-icon' : ''} />
        {refreshPending ? t('Refreshing status…') : t('Refresh status')}
      </button>
      {copyError ? <p className="setup-copy-error" role="alert">{copyError}</p> : null}
    </div>
  );
}

function shellArgument(value: string) {
  if (/^[A-Za-z0-9_./~:-]+$/.test(value)) return value;
  return `'${value.replaceAll("'", "'\\''")}'`;
}
