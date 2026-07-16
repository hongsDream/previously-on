import { Bot, CircleAlert, CircleCheck, LockKeyhole, ShieldCheck } from 'lucide-react';
import type { AiRefreshCapabilityV1 } from '../types';

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
  const copy = statusCopy[capability.status];
  const StatusIcon = capability.status === 'ready' ? CircleCheck : CircleAlert;
  return (
    <main className="settings-workspace" aria-labelledby="settings-title">
      <header className="settings-hero">
        <span>Local capabilities</span>
        <h1 id="settings-title">Settings</h1>
        <p>PreviouslyOn verifies local capabilities before exposing optional actions. History review remains local and available without AI refresh.</p>
      </header>

      <section className="settings-panel" aria-labelledby="ai-refresh-settings-title">
        <header>
          <span className="settings-icon"><Bot size={20} /></span>
          <div>
            <h2 id="ai-refresh-settings-title">AI-assisted fact refresh</h2>
            <p>Beta · explicit opt-in · user initiated only</p>
          </div>
          <span className={`capability-status capability-${capability.status}`}><StatusIcon size={13} /> {capability.status.replace('_', ' ')}</span>
        </header>
        <div className="settings-capability-body">
          <div className="capability-summary">
            <strong>{copy.title}</strong>
            <p>{capability.reason || copy.description}</p>
          </div>
          <dl>
            <div><dt>Permission profile</dt><dd><code>{capability.profileName || 'Unavailable'}</code></dd></div>
            <div><dt>Network</dt><dd>{capability.status === 'ready' ? 'Disabled' : 'Not verified'}</dd></div>
            <div><dt>Approval policy</dt><dd>{capability.status === 'ready' ? 'Never' : 'Not verified'}</dd></div>
            <div><dt>Last verified</dt><dd>{formatCheckedAt(capability.checkedAt)}</dd></div>
          </dl>
          <ul className="capability-guardrails">
            <li><ShieldCheck size={15} /><span><strong>Bounded verified input</strong>Only redacted task facts, files, tests, and Regression Contracts can enter the refresh pack.</span></li>
            <li><LockKeyhole size={15} /><span><strong>Candidate-only output</strong>Model output never becomes Evidence. You must review it before it can become a Fact Candidate.</span></li>
          </ul>
        </div>
      </section>
    </main>
  );
}

function formatCheckedAt(value?: string | null) {
  if (!value) return 'Unavailable';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? 'Unavailable' : date.toLocaleString();
}
