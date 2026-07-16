import { Clock3, FileText, List, Settings } from 'lucide-react';

const items = [
  { label: 'Tasks', icon: List },
  { label: 'Sessions', icon: Clock3 },
  { label: 'Evidence', icon: FileText },
  { label: 'Settings', icon: Settings },
];

interface BottomNavigationProps {
  activeNavigation: 'tasks' | 'sessions';
  sessionsEnabled: boolean;
  onTasksOpen: () => void;
  onSessionsOpen: () => void;
  onEvidenceOpen: () => void;
}

export function BottomNavigation({ activeNavigation, sessionsEnabled, onTasksOpen, onSessionsOpen, onEvidenceOpen }: BottomNavigationProps) {
  const actions: Record<string, (() => void) | undefined> = {
    Tasks: onTasksOpen,
    Sessions: onSessionsOpen,
    Evidence: onEvidenceOpen,
  };
  return (
    <nav className="bottom-navigation mobile-only" aria-label="Mobile navigation">
      {items.map(({ label, icon: Icon }) => (
        <button
          key={label}
          className={label.toLowerCase() === activeNavigation ? 'active' : ''}
          type="button"
          disabled={label === 'Settings' || (label === 'Sessions' && !sessionsEnabled)}
          onClick={actions[label]}
        >
          <Icon size={22} strokeWidth={1.8} />
          <span>{label}</span>
        </button>
      ))}
    </nav>
  );
}
