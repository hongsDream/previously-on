import { Check, Clipboard, RefreshCw, Terminal } from 'lucide-react';
import { useMemo, useState } from 'react';

const PATH_PLACEHOLDER = '/absolute/path/to/repository';

export function FirstRunSetup() {
  const [repositoryPath, setRepositoryPath] = useState('');
  const [copied, setCopied] = useState<number | 'all' | null>(null);
  const [copyError, setCopyError] = useState('');
  const steps = useMemo(() => setupSteps(repositoryPath), [repositoryPath]);

  const copy = async (value: string, target: number | 'all') => {
    setCopyError('');
    try {
      await navigator.clipboard.writeText(value);
      setCopied(target);
    } catch {
      setCopyError('Clipboard access was unavailable. Select and copy the steps manually.');
    }
  };

  return (
    <section className="first-run-setup" aria-labelledby="first-run-title">
      <span className="empty-lineage-mark" aria-hidden="true" />
      <h1 id="first-run-title">Connect your first repository</h1>
      <p>PreviouslyOn has not been registered for a repository on this device. Enter the repository path to prepare the commands below; this screen never runs them.</p>

      <label htmlFor="repository-path">Repository path</label>
      <input
        id="repository-path"
        type="text"
        value={repositoryPath}
        placeholder={PATH_PLACEHOLDER}
        autoComplete="off"
        spellCheck={false}
        onChange={(event) => {
          setRepositoryPath(event.target.value);
          setCopied(null);
        }}
      />

      <div className="setup-steps-heading">
        <h2>Next steps</h2>
        <button className="secondary-button" type="button" onClick={() => void copy(steps.filter((step) => step.copyable).map((step) => step.value).join('\n'), 'all')}>
          {copied === 'all' ? <Check size={15} /> : <Clipboard size={15} />}
          {copied === 'all' ? 'Copied all' : 'Copy all steps'}
        </button>
      </div>

      <ol className="setup-command-list">
        {steps.map((step, index) => (
          <li key={step.title}>
            <span className="setup-step-number">{index + 1}</span>
            <span><strong>{step.title}</strong>{step.copyable
              ? <code><Terminal size={14} aria-hidden="true" />{step.value}</code>
              : <span className="setup-manual-step">{step.value}</span>}</span>
            {step.copyable ? (
              <button className="icon-button" type="button" aria-label={`Copy ${step.title}`} onClick={() => void copy(step.value, index)}>
                {copied === index ? <Check size={16} /> : <Clipboard size={16} />}
              </button>
            ) : <span aria-hidden="true" />}
          </li>
        ))}
      </ol>
      {copyError ? <p className="setup-copy-error" role="alert">{copyError}</p> : null}
    </section>
  );
}

function setupSteps(repositoryPath: string) {
  const path = shellArgument(repositoryPath.trim() || PATH_PLACEHOLDER);
  return [
    { title: 'Register the repository', value: `previously setup codex --repo ${path}`, copyable: true },
    { title: 'Verify the local integration', value: 'previously doctor', copyable: true },
    { title: 'Restart Codex manually', value: 'Quit and reopen Codex so it loads the managed Hooks and MCP server.', copyable: false },
    { title: 'Start the first captured session', value: `previously run codex --repo ${path} --`, copyable: true },
  ];
}

export function RegisteredEmptyActions({ repositoryPath, refreshPending, onRefresh }: {
  repositoryPath: string;
  refreshPending: boolean;
  onRefresh: () => void;
}) {
  const [copied, setCopied] = useState<number | null>(null);
  const [copyError, setCopyError] = useState('');
  const path = shellArgument(repositoryPath.trim() || PATH_PLACEHOLDER);
  const commands = [
    { title: 'Start a captured Codex session', value: `previously run codex --repo ${path} --` },
    { title: 'Check the local integration', value: 'previously doctor' },
  ];

  const copy = async (value: string, index: number) => {
    setCopyError('');
    try {
      await navigator.clipboard.writeText(value);
      setCopied(index);
    } catch {
      setCopyError('Clipboard access was unavailable. Select and copy the command manually.');
    }
  };

  return (
    <div className="registered-empty-actions">
      <ol className="setup-command-list">
        {commands.map((command, index) => (
          <li key={command.title}>
            <span className="setup-step-number">{index + 1}</span>
            <span><strong>{command.title}</strong><code><Terminal size={14} aria-hidden="true" />{command.value}</code></span>
            <button className="icon-button" type="button" aria-label={`Copy ${command.title}`} onClick={() => void copy(command.value, index)}>
              {copied === index ? <Check size={16} /> : <Clipboard size={16} />}
            </button>
          </li>
        ))}
      </ol>
      <button className="secondary-button refresh-bootstrap-button" type="button" disabled={refreshPending} onClick={onRefresh}>
        <RefreshCw size={15} className={refreshPending ? 'spin-icon' : ''} />
        {refreshPending ? 'Refreshing status…' : 'Refresh status'}
      </button>
      {copyError ? <p className="setup-copy-error" role="alert">{copyError}</p> : null}
    </div>
  );
}

function shellArgument(value: string) {
  if (/^[A-Za-z0-9_./~:-]+$/.test(value)) return value;
  return `'${value.replaceAll("'", "'\\''")}'`;
}
