import { Clock3, FileText, List, Settings } from 'lucide-react';

const items = [
  { label: 'Tasks', icon: List },
  { label: 'Sessions', icon: Clock3 },
  { label: 'Evidence', icon: FileText },
  { label: 'Settings', icon: Settings },
];

export function BottomNavigation({ onEvidenceOpen }: { onEvidenceOpen: () => void }) {
  return (
    <nav className="bottom-navigation mobile-only" aria-label="Mobile navigation">
      {items.map(({ label, icon: Icon }, index) => (
        <button key={label} className={index === 0 ? 'active' : ''} type="button" disabled={label === 'Sessions' || label === 'Settings'} onClick={label === 'Evidence' ? onEvidenceOpen : undefined}>
          <Icon size={22} strokeWidth={1.8} />
          <span>{label}</span>
        </button>
      ))}
    </nav>
  );
}
