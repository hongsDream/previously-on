import { Activity, FolderGit2, ListChecks } from 'lucide-react';
import { useI18n } from '../i18n-context';
import type { RepositoryOverviewV1 } from '../types';

export function AllProjectsView({ repositories, onOpen }: {
  repositories: RepositoryOverviewV1[];
  onOpen: (repositoryId: string) => void;
}) {
  const { locale, t } = useI18n();
  return (
    <main className="all-projects-view" aria-label={t('All projects')}>
      <div className="all-projects-heading">
        <span>{t('Read-only overview')}</span>
        <h1>{t('All projects')}</h1>
        <p>{t('Compare project activity without synchronizing or mixing project history.')}</p>
      </div>
      <div className="project-summary-grid">
        {repositories.map((repository) => {
          const name = repository.primaryRoot.split('/').filter(Boolean).at(-1) ?? repository.repositoryId;
          return (
            <button
              className="project-summary-card"
              type="button"
              key={repository.repositoryId}
              onClick={() => onOpen(repository.repositoryId)}
            >
              <div className="project-summary-title">
                <FolderGit2 size={18} />
                <strong>{name}</strong>
              </div>
              <dl>
                <div><dt><ListChecks size={14} /> {t('Tasks')}</dt><dd>{repository.taskCount}</dd></div>
                <div><dt><Activity size={14} /> {t('Recent activity')}</dt><dd>{repository.recentActivityAt ? new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(repository.recentActivityAt)) : t('No activity')}</dd></div>
                <div><dt>{t('Record status')}</dt><dd>{t(repository.recordStatus)}</dd></div>
              </dl>
            </button>
          );
        })}
      </div>
      {repositories.length === 0 ? <p className="all-projects-empty">{t('No registered projects')}</p> : null}
    </main>
  );
}
