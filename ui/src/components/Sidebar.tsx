import { Clock3, FileText, GitBranch, Laptop, List, Search, Settings } from 'lucide-react';
import type { Task, TaskStatus } from '../types';
import { useI18n } from '../i18n-context';

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
  evidenceEnabled: boolean;
  onSettingsOpen: () => void;
}

export function Sidebar({ query, status, tasks, selectedTaskId, activeNavigation, onQueryChange, onStatusChange, onTaskSelect, onOverviewOpen, onEvidenceOpen, evidenceEnabled, onSettingsOpen }: SidebarProps) {
  const { t } = useI18n();
  return (
    <aside className="sidebar">
      <nav aria-label={t('Primary navigation')}>
        {navigation.map(({ label, icon: Icon }) => (
          <button
            key={label}
            className={(label === 'Tasks' && activeNavigation === 'tasks') || (label === 'Sessions' && activeNavigation === 'sessions') || (label === 'Settings' && activeNavigation === 'settings') ? 'nav-item active' : 'nav-item'}
            type="button"
            disabled={label === 'Evidence' && !evidenceEnabled}
            onClick={label === 'Tasks' ? () => onOverviewOpen('tasks') : label === 'Sessions' ? () => onOverviewOpen('sessions') : label === 'Evidence' ? onEvidenceOpen : onSettingsOpen}
          >
            <Icon size={19} strokeWidth={1.7} />
            {t(label)}
          </button>
        ))}
      </nav>

      <div className="sidebar-filter">
        <label htmlFor="task-search">{t('Find a task')}</label>
        <div className="search-field">
          <Search size={15} aria-hidden="true" />
          <input
            id="task-search"
            type="search"
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder={t('Search tasks')}
          />
        </div>
        <label htmlFor="task-status">{t('Status')}</label>
        <select id="task-status" value={status} onChange={(event) => onStatusChange(event.target.value as TaskStatus | 'all')}>
          <option value="all">{t('All tasks')}</option>
          <option value="active">{t('Active')}</option>
          <option value="completed">{t('Completed')}</option>
          <option value="abandoned">{t('Abandoned')}</option>
        </select>
        <div className="task-search-results" aria-live="polite">
          {tasks.length === 0 ? <span>{t('No matching tasks')}</span> : tasks.map((task) => (
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
              <small>{t('{status} · {count} checkpoints', { status: t(task.status === 'active' ? 'Active' : task.status === 'completed' ? 'Completed' : 'Abandoned'), count: task.checkpointIds.length })}</small>
            </button>
          ))}
        </div>
      </div>

      <div className="workspace-user" aria-label={t('Local workspace profile')}>
        <span className="avatar" aria-hidden="true"><Laptop size={16} /></span>
        <span><strong>{t('Local device')}</strong><small>{t('· No cloud account')}</small></span>
        <Chevron />
      </div>
    </aside>
  );
}

function Chevron() {
  return <span className="workspace-chevron" aria-hidden="true">⌄</span>;
}
