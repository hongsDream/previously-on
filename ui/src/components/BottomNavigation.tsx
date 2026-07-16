import { Clock3, FileText, List, Settings } from 'lucide-react';

const items = [
  { label: 'Tasks', icon: List },
  { label: 'Sessions', icon: Clock3 },
  { label: 'Evidence', icon: FileText },
  { label: 'Settings', icon: Settings },
];

interface BottomNavigationProps {
  activeNavigation: 'tasks' | 'sessions' | 'settings';
  sessionsEnabled: boolean;
  onTasksOpen: () => void;
  onSessionsOpen: () => void;
  onEvidenceOpen: () => void;
  onSettingsOpen: () => void;
}

export function BottomNavigation({ activeNavigation, sessionsEnabled, onTasksOpen, onSessionsOpen, onEvidenceOpen, onSettingsOpen }: BottomNavigationProps) {
  const actions: Record<string, (() => void) | undefined> = {
    Tasks: onTasksOpen,
    Sessions: onSessionsOpen,
    Evidence: onEvidenceOpen,
    Settings: onSettingsOpen,
  };
  return (
    <nav className="bottom-navigation mobile-only" aria-label="Mobile navigation">
      {items.map(({ label, icon: Icon }) => (
        <button
          key={label}
          className={label.toLowerCase() === activeNavigation ? 'active' : ''}
          type="button"
          disabled={label === 'Sessions' && !sessionsEnabled}
          onClick={actions[label]}
        >
          <Icon size={22} strokeWidth={1.8} />
          <span>{label}</span>
        </button>
      ))}
    </nav>
  );
}
