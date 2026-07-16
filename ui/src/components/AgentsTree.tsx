import { useMemo, useState } from 'react';
import { Bot, Check, Clipboard, GitFork, Search } from 'lucide-react';
import type { AgentV1, Task } from '../types';

interface AgentsTreeProps {
  task: Task;
  agents: AgentV1[];
}

interface AgentBranch {
  agent: AgentV1;
  children: AgentBranch[];
}

export function AgentsTree({ task, agents }: AgentsTreeProps) {
  const taskAgents = useMemo(() => agents.filter((agent) => agent.taskId === task.id), [agents, task.id]);
  const branches = useMemo(() => buildAgentTree(taskAgents), [taskAgents]);
  const [copiedId, setCopiedId] = useState('');
  const [copyMessage, setCopyMessage] = useState('');

  const copyId = async (threadId: string) => {
    try {
      if (!navigator.clipboard?.writeText) throw new Error('Clipboard API unavailable');
      await navigator.clipboard.writeText(threadId);
      setCopiedId(threadId);
      setCopyMessage('Codex task ID copied.');
    } catch {
      setCopiedId('');
      setCopyMessage('Copy is unavailable. Select the visible task ID instead.');
    }
  };

  return (
    <section className="agents-tree-panel" aria-labelledby="agents-tree-title">
      <header>
        <div>
          <span className="task-integrity-kicker">Same-device observation</span>
          <h2 id="agents-tree-title"><GitFork size={16} /> Local agents</h2>
          <p>Observed Codex task ancestry only. This is not cloud sync, team membership, or orchestration control.</p>
        </div>
        <span>{taskAgents.length} observed</span>
      </header>

      {taskAgents.length ? (
        <ul className="agents-tree" role="tree" aria-label={`Agents observed for ${task.title}`}>
          {branches.map((branch) => <AgentTreeItem key={branch.agent.id} branch={branch} level={1} copiedId={copiedId} onCopy={copyId} />)}
        </ul>
      ) : <p className="agents-empty">No local App Server agent lineage is linked to this task.</p>}

      <aside className="find-codex-guide" aria-labelledby="find-codex-title">
        <Search size={16} />
        <span><strong id="find-codex-title">Find in Codex</strong>Copy the unique task ID, then paste it into Codex task search. No public desktop focus or open interface is available, so PreviouslyOn does not create a private deep link or an Open button.</span>
      </aside>
      <p className="sr-only" aria-live="polite">{copyMessage}</p>
    </section>
  );
}

function AgentTreeItem({
  branch,
  level,
  copiedId,
  onCopy,
}: {
  branch: AgentBranch;
  level: number;
  copiedId: string;
  onCopy: (threadId: string) => Promise<void>;
}) {
  const { agent, children } = branch;
  return (
    <li role="treeitem" aria-level={level} aria-expanded={children.length ? true : undefined}>
      <article className={`agent-card association-${agent.associationState}`}>
        <header>
          <span className="agent-avatar"><Bot size={15} /></span>
          <span className="agent-identity">
            <strong>{agent.name || `${roleLabel(agent.role)} · ${shortId(agent.threadId)}`}</strong>
            <small>{sourceKindLabel(agent.sourceKind)} · {roleLabel(agent.role)}</small>
          </span>
          <span className={`agent-status agent-status-${agent.status}`}>{agent.status}</span>
          <span className={`agent-association association-${agent.associationState}`}>{agent.associationState}</span>
        </header>
        <div className="agent-task-id">
          <span><small>Codex task ID</small><code title={agent.threadId}>{agent.threadId}</code></span>
          <button className="secondary-button" type="button" onClick={() => void onCopy(agent.threadId)} aria-label={`Copy Codex task ID ${agent.threadId}`}>
            {copiedId === agent.threadId ? <Check size={13} /> : <Clipboard size={13} />} {copiedId === agent.threadId ? 'Copied' : 'Copy ID'}
          </button>
        </div>
        {agent.degradedReason ? <p className="agent-degraded-reason">{agent.degradedReason}</p> : null}
        {agent.outputSummary ? <p className="agent-output-summary">{agent.outputSummary}</p> : null}
        {agent.files.length || agent.tests.length ? (
          <dl className="agent-observations">
            <div><dt>Files</dt><dd>{agent.files.length ? agent.files.map((file) => <code key={file}>{file}</code>) : 'None observed'}</dd></div>
            <div><dt>Tests</dt><dd>{agent.tests.length ? agent.tests.map((test) => <code key={test}>{test}</code>) : 'None observed'}</dd></div>
          </dl>
        ) : null}
      </article>
      {children.length ? <ul role="group">{children.map((child) => <AgentTreeItem key={child.agent.id} branch={child} level={level + 1} copiedId={copiedId} onCopy={onCopy} />)}</ul> : null}
    </li>
  );
}

function buildAgentTree(agents: AgentV1[]) {
  const sorted = [...agents].sort((left, right) => left.observedAt.localeCompare(right.observedAt) || left.threadId.localeCompare(right.threadId));
  const byThreadId = new Map(sorted.map((agent) => [agent.threadId, agent]));
  const children = new Map<string, AgentV1[]>();
  for (const agent of sorted) {
    if (agent.parentThreadId && byThreadId.has(agent.parentThreadId) && agent.parentThreadId !== agent.threadId) {
      const siblings = children.get(agent.parentThreadId) ?? [];
      siblings.push(agent);
      children.set(agent.parentThreadId, siblings);
    }
  }
  const roots = sorted.filter((agent) => !agent.parentThreadId || !byThreadId.has(agent.parentThreadId) || agent.parentThreadId === agent.threadId);
  const visited = new Set<string>();
  const branch = (agent: AgentV1): AgentBranch => {
    if (visited.has(agent.threadId)) return { agent, children: [] };
    visited.add(agent.threadId);
    return { agent, children: (children.get(agent.threadId) ?? []).map(branch) };
  };
  const result = roots.map(branch);
  for (const agent of sorted) {
    if (!visited.has(agent.threadId)) result.push(branch(agent));
  }
  return result;
}

function shortId(value: string) {
  return value.length > 18 ? `${value.slice(0, 9)}…${value.slice(-6)}` : value;
}

function roleLabel(value: string) {
  return value.replaceAll('_', ' ').replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function sourceKindLabel(value: AgentV1['sourceKind']) {
  return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/\b\w/g, (letter) => letter.toUpperCase());
}
