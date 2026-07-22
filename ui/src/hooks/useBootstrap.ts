import { useCallback, useEffect, useRef, useState } from 'react';
import { fallbackData } from '../data/fallback';
import {
  ApiUnavailableError,
  fetchBootstrap,
  fetchRepositoryOverview,
  syncCodexRepository,
  toUiError,
  type UiError,
} from '../lib/api';
import { readPreferences, updatePreferences } from '../lib/preferences';
import {
  emptyWorkspaceSelection,
  normalizeBootstrap,
  resolveTaskSelection,
  type WorkspaceSelectionIds,
} from '../lib/workspace';
import type { BootstrapData, CodexImportReportV1, RepositoryOverviewV1 } from '../types';

const AUTO_SYNC_KEY_PREFIX = 'previously-on:codex-auto-sync:v1:';

export function useBootstrap() {
  const [data, setData] = useState<BootstrapData | null>(null);
  const [repositories, setRepositories] = useState<RepositoryOverviewV1[]>([]);
  const [selectedRepositoryId, setSelectedRepositoryId] = useState<string | null>(null);
  const [bootstrapRepositoryId, setBootstrapRepositoryId] = useState<string | null>(null);
  const [allProjects, setAllProjects] = useState(false);
  const [syncReports, setSyncReports] = useState<Record<string, CodexImportReportV1>>({});
  const [syncPending, setSyncPending] = useState(false);
  const [syncError, setSyncError] = useState<UiError | null>(null);
  const [offlineFallback, setOfflineFallback] = useState(false);
  const [fatalError, setFatalError] = useState<UiError | null>(null);
  const [selection, setSelection] = useState<WorkspaceSelectionIds>(emptyWorkspaceSelection);
  const loadVersion = useRef(0);
  const selectedRepositoryIdRef = useRef<string | null>(null);
  const activeController = useRef<AbortController | null>(null);
  const autoSyncStarted = useRef(new Set<string>());

  const installBootstrap = useCallback((
    next: BootstrapData,
    preferred: Partial<WorkspaceSelectionIds> = emptyWorkspaceSelection,
  ) => {
    const normalized = normalizeBootstrap(next);
    const resolved = resolveTaskSelection(
      normalized,
      preferred.taskId,
      preferred.checkpointId,
      preferred.evidenceId,
    );
    setData(normalized);
    setSelection(resolved);
  }, []);

  const reserveAutomaticSync = useCallback((repositoryId: string) => {
    if (autoSyncStarted.current.has(repositoryId)) return false;
    const key = `${AUTO_SYNC_KEY_PREFIX}${repositoryId}`;
    try {
      if (sessionStorage.getItem(key) === 'started') return false;
      sessionStorage.setItem(key, 'started');
    } catch {
      // The in-memory set still enforces once-per-mounted browser session.
    }
    autoSyncStarted.current.add(repositoryId);
    return true;
  }, []);

  const synchronize = useCallback(async (
    repositoryId: string,
    version: number,
    automatic: boolean,
  ) => {
    if (automatic && !reserveAutomaticSync(repositoryId)) return;
    setSyncPending(true);
    setSyncError(null);
    try {
      const report = await syncCodexRepository(repositoryId);
      setSyncReports((current) => ({ ...current, [repositoryId]: report }));
      if (loadVersion.current !== version || selectedRepositoryIdRef.current !== repositoryId) return;
      const refreshed = await fetchBootstrap(repositoryId);
      if (loadVersion.current === version) installBootstrap(refreshed);
    } catch (error) {
      if (loadVersion.current === version && selectedRepositoryIdRef.current === repositoryId) {
        setSyncError(toUiError(error, 'Codex synchronization failed.'));
      }
    } finally {
      if (loadVersion.current === version && selectedRepositoryIdRef.current === repositoryId) {
        setSyncPending(false);
      }
    }
  }, [installBootstrap, reserveAutomaticSync]);

  const loadRepository = useCallback((repositoryId: string, automaticSync: boolean) => {
    activeController.current?.abort();
    const controller = new AbortController();
    activeController.current = controller;
    const version = ++loadVersion.current;
    setAllProjects(false);
    setSelectedRepositoryId(repositoryId);
    setBootstrapRepositoryId(repositoryId);
    selectedRepositoryIdRef.current = repositoryId;
    setData(null);
    setFatalError(null);
    setSyncError(null);
    setSyncPending(false);
    updatePreferences({ repositoryId });
    fetchBootstrap(repositoryId, controller.signal)
      .then((bootstrap) => {
        if (loadVersion.current !== version) return;
        installBootstrap(bootstrap);
        if (automaticSync) void synchronize(repositoryId, version, true);
      })
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === 'AbortError') return;
        if (loadVersion.current !== version || selectedRepositoryIdRef.current !== repositoryId) return;
        setFatalError(toUiError(error, 'The local API returned an invalid response.'));
      });
  }, [installBootstrap, synchronize]);

  useEffect(() => {
    const controller = new AbortController();
    activeController.current = controller;
    const version = ++loadVersion.current;
    const installLegacyBootstrap = (legacyBootstrap: BootstrapData) => {
      if (!legacyBootstrap.repository || !Array.isArray(legacyBootstrap.tasks)) {
        throw new Error('The local project bootstrap returned an invalid response.');
      }
      const repositoryId = legacyBootstrap.tasks[0]?.repositoryId ?? legacyBootstrap.repository.name;
      selectedRepositoryIdRef.current = repositoryId;
      setSelectedRepositoryId(repositoryId);
      setBootstrapRepositoryId(null);
      setRepositories([{
        repositoryId,
        primaryRoot: legacyBootstrap.repository.path,
        taskCount: legacyBootstrap.tasks.length,
        recentActivityAt: legacyBootstrap.tasks[0]?.updatedAt,
        recordStatus: legacyBootstrap.repository.state === 'degraded' ? 'degraded' : legacyBootstrap.tasks.length > 0 ? 'ready' : 'empty',
      }]);
      installBootstrap(legacyBootstrap);
    };

    const loadLegacyBootstrap = async () => {
      const bootstrap = await fetchBootstrap(undefined, controller.signal);
      if (loadVersion.current === version) installLegacyBootstrap(bootstrap);
    };

    fetchRepositoryOverview(controller.signal)
      .then((overview) => {
        if (loadVersion.current !== version) return;
        if (!Array.isArray(overview.repositories)) {
          const legacyBootstrap = overview as unknown as BootstrapData;
          if (legacyBootstrap.repository && Array.isArray(legacyBootstrap.tasks)) {
            installLegacyBootstrap(legacyBootstrap);
            return;
          }
          return loadLegacyBootstrap();
        }
        const available = overview.repositories;
        setRepositories(available);
        const preferred = readPreferences().repositoryId;
        const repositoryId = available.some((repository) => repository.repositoryId === preferred)
          ? preferred!
          : available[0]?.repositoryId;
        if (!repositoryId) {
          return fetchBootstrap(undefined, controller.signal).then((bootstrap) => {
            if (loadVersion.current === version) installBootstrap(bootstrap);
          });
        }
        setSelectedRepositoryId(repositoryId);
        setBootstrapRepositoryId(repositoryId);
        selectedRepositoryIdRef.current = repositoryId;
        updatePreferences({ repositoryId });
        return fetchBootstrap(repositoryId, controller.signal).then((bootstrap) => {
          if (loadVersion.current !== version) return;
          installBootstrap(bootstrap);
          void synchronize(repositoryId, version, true);
        });
      })
      .catch(async () => {
        if (loadVersion.current !== version) return;
        try {
          await loadLegacyBootstrap();
          return;
        } catch (error: unknown) {
          if (error instanceof DOMException && error.name === 'AbortError') return;
          if (!(error instanceof ApiUnavailableError)) {
            setFatalError(toUiError(error, 'The local API returned an invalid response.'));
            return;
          }
        }
        setOfflineFallback(true);
        const repositoryId = fallbackData.tasks[0]?.repositoryId ?? fallbackData.repository.name;
        setSelectedRepositoryId(repositoryId);
        setBootstrapRepositoryId(null);
        selectedRepositoryIdRef.current = repositoryId;
        setRepositories([{
          repositoryId,
          primaryRoot: fallbackData.repository.path,
          taskCount: fallbackData.tasks.length,
          recentActivityAt: fallbackData.tasks[0]?.updatedAt,
          recordStatus: 'ready',
        }]);
        installBootstrap(fallbackData);
      });
    return () => controller.abort();
  }, [installBootstrap, synchronize]);

  const showAllProjects = useCallback(() => {
    activeController.current?.abort();
    loadVersion.current += 1;
    setAllProjects(true);
    setSyncPending(false);
    setSyncError(null);
  }, []);

  const manualSync = useCallback(() => {
    if (!selectedRepositoryId || allProjects || offlineFallback) return;
    void synchronize(selectedRepositoryId, loadVersion.current, false);
  }, [allProjects, offlineFallback, selectedRepositoryId, synchronize]);

  return {
    data,
    setData,
    repositories,
    selectedRepositoryId,
    bootstrapRepositoryId,
    allProjects,
    selectRepository: (repositoryId: string) => loadRepository(repositoryId, true),
    showAllProjects,
    syncReport: selectedRepositoryId ? syncReports[selectedRepositoryId] : undefined,
    syncPending,
    syncError,
    manualSync,
    offlineFallback,
    fatalError,
    selection,
    setSelection,
    installBootstrap,
  };
}
