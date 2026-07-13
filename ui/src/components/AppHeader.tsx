import { useState } from 'react';
import { Box, ChevronDown, ChevronRight, Download, GitBranch, Headphones, MoreVertical, Trash2 } from 'lucide-react';
import { Brand } from './Brand';
import type { BootstrapData } from '../types';

interface AppHeaderProps {
  repository: BootstrapData['repository'];
  onPreview: () => void;
  onExport: () => void;
  onPurge: () => void;
  actionsDisabled?: boolean;
}

export function AppHeader({ repository, onPreview, onExport, onPurge, actionsDisabled = false }: AppHeaderProps) {
  const [menuOpen, setMenuOpen] = useState(false);

  return (
    <header className="app-header">
      <div className="app-header-brand">
        <span className="window-controls" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
        <Brand />
      </div>

      <div className="connection-strip" aria-label="Repository connection">
        <Headphones size={15} />
        <span>{repository.connected ? 'Connected' : 'Disconnected'}</span>
        <span className={`health-dot health-${repository.captureHealth}`} aria-hidden="true" />
        <span className="repo-path">{repository.path}</span>
        <span className="repo-branch"><GitBranch size={13} /> {repository.branch}</span>
      </div>

      <button className="mobile-repository" type="button" disabled>
        <span className="github-disc" aria-hidden="true">GH</span>
        <span>{repository.name}</span>
        <ChevronDown size={17} />
      </button>
      <button className="mobile-capture-health" type="button" disabled aria-label={`Capture health: ${repository.captureHealth}`}>
        <i className={`health-dot health-${repository.captureHealth}`} />
        <span>{repository.captureHealth === 'good' ? 'Healthy' : repository.captureHealth}</span>
        <ChevronRight size={15} />
      </button>

      <div className="header-actions">
        <button className="capture-health" type="button" disabled aria-label={`Capture health: ${repository.captureHealth}`}>
          <span>Capture health</span>
          <i className={`health-dot health-${repository.captureHealth}`} />
          <strong>{repository.captureHealth === 'good' ? 'Good' : repository.captureHealth}</strong>
          <ChevronRight size={15} />
        </button>
        <button className="primary-button preview-button" type="button" onClick={onPreview}>
          <Box size={15} />
          Preview context pack
        </button>
        <div className="header-menu">
          <button
            className="icon-button"
            type="button"
            aria-label="More options"
            aria-expanded={menuOpen}
            aria-haspopup="menu"
            onClick={() => setMenuOpen((open) => !open)}
          ><MoreVertical size={18} /></button>
          {menuOpen ? (
            <div className="header-menu-popover" role="menu">
              <button type="button" role="menuitem" disabled={actionsDisabled} onClick={() => { setMenuOpen(false); onExport(); }}>
                <Download size={15} /> Export JSON
              </button>
              <button className="danger-menu-item" type="button" role="menuitem" disabled={actionsDisabled || !repository.connected} onClick={() => { setMenuOpen(false); onPurge(); }}>
                <Trash2 size={15} /> Delete repository data
              </button>
            </div>
          ) : null}
        </div>
      </div>
    </header>
  );
}
