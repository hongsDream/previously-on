import { Clock3, FileText, GitBranch, List, Search, Settings } from 'lucide-react';
import type { Task, TaskStatus } from '../types';

const navigation = [
  { label: 'Tasks', icon: List },
  { label: 'Sessions', icon: Clock3 },
  { label: 'Evidence', icon: FileText },
  { label: 'Settings', icon: Settings },
];

interface SidebarProps {
  query: string;
  status: TaskStatus | 'all';
  tasks: Task[];
  selectedTaskId: string;
  onQueryChange: (query: string) => void;
  onStatusChange: (status: TaskStatus | 'all') => void;
  onTaskSelect: (taskId: string) => void;
  activeNavigation: 'tasks' | 'sessions' | 'task' | 'settings';
  onOverviewOpen: (focus: 'tasks' | 'sessions') => void;
  onEvidenceOpen: () => void;
  onSettingsOpen: () => void;
}

export function Sidebar({ query, status, tasks, selectedTaskId, activeNavigation, onQueryChange, onStatusChange, onTaskSelect, onOverviewOpen, onEvidenceOpen, onSettingsOpen }: SidebarProps) {
  return (
    <aside className="sidebar">
      <nav aria-label="Primary navigation">
        {navigation.map(({ label, icon: Icon }) => (
          <button
            key={label}
            className={(label === 'Tasks' && activeNavigation === 'tasks') || (label === 'Sessions' && activeNavigation === 'sessions') || (label === 'Settings' && activeNavigation === 'settings') ? 'nav-item active' : 'nav-item'}
            type="button"
            onClick={label === 'Tasks' ? () => onOverviewOpen('tasks') : label === 'Sessions' ? () => onOverviewOpen('sessions') : label === 'Evidence' ? onEvidenceOpen : onSettingsOpen}
          >
            <Icon size={19} strokeWidth={1.7} />
            {label}
          </button>
        ))}
      </nav>

      <div className="sidebar-filter">
        <label htmlFor="task-search">Find a task</label>
        <div className="search-field">
          <Search size={15} aria-hidden="true" />
          <input
            id="task-search"
            type="search"
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="Search tasks"
          />
        </div>
        <label htmlFor="task-status">Status</label>
        <select id="task-status" value={status} onChange={(event) => onStatusChange(event.target.value as TaskStatus | 'all')}>
          <option value="all">All tasks</option>
          <option value="active">Active</option>
          <option value="completed">Completed</option>
          <option value="abandoned">Abandoned</option>
        </select>
        <div className="task-search-results" aria-live="polite">
          {tasks.length === 0 ? <span>No matching tasks</span> : tasks.map((task) => (
            <button
              key={task.id}
              className={task.id === selectedTaskId ? 'active' : ''}
              type="button"
              onClick={() => onTaskSelect(task.id)}
            >
              <strong>{task.title}</strong>
              <span className="task-result-repository">
                <span>{task.codebase.repositoryName}</span>
                <code><GitBranch size={10} /> {task.codebase.branch}</code>
              </span>
              <small>{task.status} · {task.checkpointIds.length} checkpoints</small>
            </button>
          ))}
        </div>
      </div>

      <div className="workspace-user" aria-label="Local workspace profile">
        <span className="avatar">JD</span>
        <span><strong>jdoe</strong><small><i className="health-dot health-good" /> Local workspace</small></span>
        <Chevron />
      </div>
    </aside>
  );
}

function Chevron() {
  return <span className="workspace-chevron" aria-hidden="true">⌄</span>;
}
