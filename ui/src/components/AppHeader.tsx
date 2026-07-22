import { useState } from 'react';
import { Box, ChevronDown, ChevronRight, Download, GitBranch, Headphones, MoreVertical, RefreshCw, Trash2 } from 'lucide-react';
import { Brand } from './Brand';
import { useI18n } from '../i18n-context';
import type { BootstrapData, RepositoryOverviewV1 } from '../types';

interface AppHeaderProps {
  repository?: BootstrapData['repository'];
  onPreview: () => void;
  onExport: () => void;
  onPurge: () => void;
  actionsDisabled?: boolean;
  previewDisabled?: boolean;
  repositories: RepositoryOverviewV1[];
  selectedRepositoryId: string | null;
  allProjects: boolean;
  syncPending: boolean;
  onRepositorySelect: (repositoryId: string) => void;
  onAllProjects: () => void;
  onSync: () => void;
}

const repositoryStateCopy: Record<BootstrapData['repository']['state'], string> = {
  unregistered: 'Not registered',
  'registered-empty': 'Registered · awaiting first checkpoint',
  active: 'Active',
  degraded: 'Degraded',
};

export function AppHeader({ repository, onPreview, onExport, onPurge, actionsDisabled = false, previewDisabled = false, repositories, selectedRepositoryId, allProjects, syncPending, onRepositorySelect, onAllProjects, onSync }: AppHeaderProps) {
  const { t } = useI18n();
  const [menuOpen, setMenuOpen] = useState(false);
  const isUnregistered = !repository || repository.state === 'unregistered';
  const stateCopy = repository ? t(repositoryStateCopy[repository.state]) : t('All projects');
  const captureHealth = repository?.captureHealth ?? 'offline';

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

      <div className="connection-strip" aria-label={t('Repository connection')}>
        <select
          className="project-selector"
          aria-label={t('Project selector')}
          value={allProjects ? '__all__' : selectedRepositoryId ?? '__none__'}
          onChange={(event) => event.target.value === '__all__' ? onAllProjects() : event.target.value === '__none__' ? undefined : onRepositorySelect(event.target.value)}
        >
          <option value="__all__">{t('All projects')}</option>
          {repositories.length === 0 ? <option value="__none__">{t('No repository')}</option> : null}
          {repositories.map((project) => <option key={project.repositoryId} value={project.repositoryId}>{project.primaryRoot.split('/').filter(Boolean).at(-1) ?? project.repositoryId}</option>)}
        </select>
        <Headphones size={15} />
        <span>{stateCopy}</span>
        <span className={`health-dot health-${captureHealth}`} aria-hidden="true" />
        {isUnregistered ? null : <span className="repo-path">{repository?.path}</span>}
        {isUnregistered ? null : <span className="repo-branch"><GitBranch size={13} /> {repository?.branch}</span>}
      </div>

      <button className="mobile-repository" type="button" disabled>
        <span className="github-disc" aria-hidden="true">GH</span>
        <span>{allProjects ? t('All projects') : isUnregistered ? t('No repository') : repository?.name}</span>
        <ChevronDown size={17} />
      </button>
      <button className="mobile-capture-health" type="button" disabled aria-label={t('Repository state: {state}', { state: stateCopy })}>
        <i className={`health-dot health-${captureHealth}`} />
        <span>{stateCopy}</span>
        <ChevronRight size={15} />
      </button>

      <div className="header-actions">
        {!allProjects && selectedRepositoryId ? (
          <button className="secondary-button sync-button" type="button" disabled={actionsDisabled || syncPending} onClick={onSync}>
            <RefreshCw size={15} className={syncPending ? 'spinning' : undefined} />
            {t(syncPending ? 'Synchronizing…' : 'Sync Codex app history')}
          </button>
        ) : null}
        <button className="capture-health" type="button" disabled aria-label={t('Repository state: {state}', { state: stateCopy })}>
          <span>{t('Repository state')}</span>
          <i className={`health-dot health-${captureHealth}`} />
          <strong>{stateCopy}</strong>
          <ChevronRight size={15} />
        </button>
        <button className="primary-button preview-button" type="button" disabled={previewDisabled} onClick={onPreview}>
          <Box size={15} />
          {t('Preview context pack')}
        </button>
        <div className="header-menu">
          <button
            className="icon-button"
            type="button"
            aria-label={t('More options')}
            aria-expanded={menuOpen}
            aria-haspopup="menu"
            onClick={() => setMenuOpen((open) => !open)}
          ><MoreVertical size={18} /></button>
          {menuOpen ? (
            <div className="header-menu-popover" role="menu">
              <button type="button" role="menuitem" disabled={actionsDisabled} onClick={() => { setMenuOpen(false); onExport(); }}>
                <Download size={15} /> {t('Export JSON')}
              </button>
              <button className="danger-menu-item" type="button" role="menuitem" disabled={actionsDisabled || !repository?.connected} onClick={() => { setMenuOpen(false); onPurge(); }}>
                <Trash2 size={15} /> {t('Delete repository data')}
              </button>
            </div>
          ) : null}
        </div>
      </div>
    </header>
  );
}
