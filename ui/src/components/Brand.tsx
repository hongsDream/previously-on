import { Circle, GitFork } from 'lucide-react';

export function Brand() {
  return (
    <div className="brand" aria-label="PreviouslyOn">
      <span className="brand-mark" aria-hidden="true">
        <GitFork size={18} strokeWidth={2.4} />
        <Circle className="brand-dot brand-dot-a" size={5} fill="currentColor" />
        <Circle className="brand-dot brand-dot-b" size={5} fill="currentColor" />
      </span>
      <span>PreviouslyOn</span>
    </div>
  );
}
